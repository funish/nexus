use anyhow::Result;
use rolldown::plugin::{
    HookLoadArgs, HookLoadOutput, HookResolveIdArgs, HookResolveIdOutput, HookUsage, Plugin,
    PluginContext,
};
use rolldown::{BundlerBuilder, BundlerOptions, InputItem, OutputFormat, Platform};
use rolldown_common::{ImportKind, ResolvedExternal};
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;

use crate::cdn::entry::resolve_esm_entry;
use crate::storage::SharedStorage;

/// Facade namespace for `require(external)` → ESM conversion. Rolldown keeps
/// require semantics for externals (generating a `__require` shim that throws in
/// browsers), so we turn each CJS require of an external into a virtual CJS module
/// that re-exports the external's ESM namespace — the same trick as rolldown's
/// built-in esmExternalRequirePlugin.
const EXTERNAL_REQUIRE_FACADE: &str = "builtin:esm-external-require:";

/// Prefix for virtual module ids backed by storage. The `scheme://` form is
/// opaque to rolldown's filesystem resolver (a bare storage key looks like a
/// relative path and gets mangled); the rest of the id is the storage key.
const VIRTUAL_PREFIX: &str = "virtual://";

/// Node builtin modules exposed as bare specifiers (without the `node:` prefix).
/// These can never resolve to an npm package, so they stay external as-is.
const NODE_BUILTINS: &[&str] = &[
    "assert",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "domain",
    "events",
    "fs",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "string_decoder",
    "timers",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

#[derive(Clone)]
pub struct EsmBundleOptions {
    pub package_name: String,
    pub version: String,
}

pub async fn bundle_esm_package(
    storage: &SharedStorage,
    options: &EsmBundleOptions,
) -> Result<String> {
    let cache_base = format!("cdn/npm/{}/{}", options.package_name, options.version);
    let esm_key = format!("{cache_base}/+esm");

    if let Some(cached) = storage.get_raw(&esm_key).await {
        return String::from_utf8(cached)
            .map_err(|e| anyhow::anyhow!("cached bundle is not valid UTF-8: {e}"));
    }

    // Single-flight: concurrent +esm requests for the same package share one
    // bundling run; followers re-read the cached bundle afterward.
    let storage_for_fn = storage.clone();
    let opts = options.clone();
    let key = esm_key.clone();
    crate::utils::singleflight::run_once(&esm_key, || {
        let storage = storage_for_fn.clone();
        let opts = opts.clone();
        let key = key.clone();
        async move {
            if storage.get_raw(&key).await.is_some() {
                return;
            }
            match build_bundle(&storage, &opts).await {
                Ok(code) => {
                    if let Err(e) = storage.set_raw(&key, code.as_bytes()).await {
                        tracing::warn!("Failed to cache ESM bundle {key}: {e}");
                    }
                }
                Err(e) => tracing::warn!(
                    "ESM bundle failed for {}@{}: {e}",
                    opts.package_name,
                    opts.version
                ),
            }
        }
    })
    .await;

    storage
        .get_raw(&esm_key)
        .await
        .map(|d| {
            String::from_utf8(d)
                .map_err(|e| anyhow::anyhow!("cached bundle is not valid UTF-8: {e}"))
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ESM bundle unavailable for {}@{}",
                options.package_name,
                options.version
            )
        })?
}

async fn build_bundle(storage: &SharedStorage, options: &EsmBundleOptions) -> Result<String> {
    // Cap concurrent bundles: rolldown is CPU/memory-heavy. Without this a
    // cold-start burst of distinct packages can starve CPU and OOM the process.
    let _bundle_permit = crate::utils::concurrency::BUNDLE_SEMAPHORE
        .acquire()
        .await
        .unwrap();

    let cache_base = format!("cdn/npm/{}/{}", options.package_name, options.version);

    let meta = storage.get_meta(&cache_base).await.ok_or_else(|| {
        anyhow::anyhow!(
            "Package {}@{} is not cached yet",
            options.package_name,
            options.version
        )
    })?;

    let files: HashSet<String> = meta
        .files
        .unwrap_or_default()
        .into_iter()
        .map(|f| f.name)
        .collect();

    // Read package.json for dependencies
    let pkg_json_data = storage
        .get_raw(&format!("{cache_base}/package.json"))
        .await
        .ok_or_else(|| {
            anyhow::anyhow!(
                "package.json not found for {}@{}",
                options.package_name,
                options.version
            )
        })?;
    let pkg_json: serde_json::Value = serde_json::from_slice(&pkg_json_data)?;

    // Resolve the entry against the package's own file list: the package.json
    // ESM entry fields first, then common fallback names, then index.json for
    // data-only packages (a bare JSON bundle exports its payload as default).
    // Each candidate is verified against the files actually in the package.
    let entry = [resolve_esm_entry(&pkg_json)]
        .into_iter()
        .flatten()
        .chain(
            crate::cdn::entry::ENTRY_FALLBACKS
                .iter()
                .map(|s| (*s).to_string()),
        )
        .chain(std::iter::once("index.json".to_string()))
        .find_map(|cand| resolve_in_files(&files, &cand))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No servable entry point for {}@{}",
                options.package_name,
                options.version
            )
        })?;

    // Collect dependency entries once: each entry with a resolvable range also
    // drives a concurrent version fetch for import URL resolution (esm.sh
    // behavior — newest published version satisfying the range). Fetches hit the
    // metadata cache, so repeated bundles of the same dep tree are cheap.
    let dep_entries: Vec<(String, Option<String>)> = ["dependencies", "peerDependencies"]
        .iter()
        .filter_map(|f| pkg_json[*f].as_object())
        .flatten()
        .map(|(name, range)| (name.clone(), range.as_str().map(String::from)))
        .collect();

    let mut tasks = tokio::task::JoinSet::new();
    for (name, range_str) in &dep_entries {
        if let Some(rs) = range_str
            && let Ok(req) = rs.parse::<node_semver::Range>()
        {
            let storage = storage.clone();
            let name = name.clone();
            tasks.spawn(async move {
                let v = latest_version_satisfying(&storage, &name, &req).await;
                (name, v)
            });
        }
    }
    let mut dep_versions: HashMap<String, String> = HashMap::new();
    while let Some(res) = tasks.join_next().await {
        if let Ok((name, Some(v))) = res {
            dep_versions.insert(name, v);
        }
    }

    // Node subpath imports (`#foo` specifiers map through package.json "imports").
    let imports: HashMap<String, String> = pkg_json["imports"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| conditions_target(v).map(|t| (k.clone(), t)))
                .collect()
        })
        .unwrap_or_default();

    let plugin = StoragePlugin {
        cache_base,
        files,
        dep_versions,
        imports,
        storage: storage.clone(),
    };

    let bundler_options = BundlerOptions {
        input: Some(vec![InputItem {
            name: Some("entry".to_string()),
            import: format!("{}{}/{}", VIRTUAL_PREFIX, plugin.cache_base, entry),
        }]),
        format: Some(OutputFormat::Esm),
        platform: Some(Platform::Browser),
        minify: Some(rolldown::RawMinifyOptions::Bool(true)),
        ..Default::default()
    };

    let mut bundler = BundlerBuilder::default()
        .with_options(bundler_options)
        .with_plugins(vec![<StoragePlugin as Plugin>::new_shared(plugin)])
        .build()?;
    let output = bundler.generate().await?;
    bundler.close().await?;

    for out in &output.assets {
        if let rolldown_common::Output::Chunk(chunk) = out {
            return Ok(chunk.code.to_string());
        }
    }
    anyhow::bail!("Rolldown produced no output")
}

/// Rolldown plugin that serves package files straight from the storage layer and
/// rewrites bare imports to `/cdn/npm/...` URLs — no temp dir, no regex pass over
/// the output. Relative imports resolve against the cached file list with npm
/// extension/index fallbacks, so the bundle never touches the filesystem.
struct StoragePlugin {
    /// Storage prefix of the package being bundled, e.g. `cdn/npm/foo/1.2.3`.
    /// Doubles as the virtual module namespace: a module id is the storage key.
    cache_base: String,
    /// File names inside the package (relative to the package root).
    files: HashSet<String>,
    /// Dependency name → resolved version for import URL rewriting.
    dep_versions: HashMap<String, String>,
    /// package.json `imports` mappings for `#foo` subpath import specifiers.
    imports: HashMap<String, String>,
    storage: SharedStorage,
}

impl fmt::Debug for StoragePlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoragePlugin")
            .field("cache_base", &self.cache_base)
            .field("files", &self.files.len())
            .finish_non_exhaustive()
    }
}

/// `/`-normalized join of a virtual directory and a relative specifier.
fn join_virtual(dir: &str, spec: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    let joined = format!("{dir}/{spec}");
    for seg in joined.split('/') {
        match seg {
            ".." => {
                segments.pop();
            }
            "." | "" => {}
            s => segments.push(s),
        }
    }
    segments.join("/")
}

/// Match a package-relative name against the cached file list, with npm-style
/// extension and index fallbacks (`main: "./index"` packages are common).
/// Returns the matching file name.
fn resolve_in_files(files: &HashSet<String>, name: &str) -> Option<String> {
    let name = name.trim_start_matches("./");
    [
        name.to_string(),
        format!("{name}.js"),
        format!("{name}.mjs"),
        format!("{name}.cjs"),
        format!("{name}.json"),
        format!("{name}/index.js"),
        format!("{name}/index.mjs"),
        format!("{name}/index.cjs"),
    ]
    .into_iter()
    .find(|cand| files.contains(cand))
}

/// Flatten an `imports`/`exports`-style value to its file target: a plain string,
/// or a conditions object preferring browser > import > default.
fn conditions_target(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        obj @ serde_json::Value::Object(_) => ["browser", "import", "default"]
            .iter()
            .find_map(|c| obj.get(*c).and_then(conditions_target)),
        _ => None,
    }
}

impl StoragePlugin {
    /// Resolve a relative specifier from a package-internal importer against the
    /// cached file list. Returns a full virtual module id.
    fn resolve_relative(&self, importer: &str, spec: &str) -> Option<String> {
        let importer = importer.strip_prefix(VIRTUAL_PREFIX)?;
        let dir = importer.rsplit_once('/').map(|(d, _)| d)?;
        let target = join_virtual(dir, spec);
        let rest = target
            .strip_prefix(&self.cache_base)?
            .trim_start_matches('/');
        resolve_in_files(&self.files, rest)
            .map(|cand| format!("{VIRTUAL_PREFIX}/{}/{cand}", self.cache_base))
    }

    fn cdn_url(&self, pkg: &str, subpath: &str) -> String {
        match self.dep_versions.get(pkg) {
            Some(v) if subpath.is_empty() => format!("/cdn/npm/{pkg}@{v}/+esm"),
            Some(v) => format!("/cdn/npm/{pkg}@{v}/{subpath}"),
            None if subpath.is_empty() => format!("/cdn/npm/{pkg}/+esm"),
            None => format!("/cdn/npm/{pkg}/{subpath}"),
        }
    }
}

impl Plugin for StoragePlugin {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("nexus-storage")
    }

    async fn resolve_id(
        &self,
        _ctx: &PluginContext,
        args: &HookResolveIdArgs<'_>,
    ) -> rolldown::plugin::HookResolveIdReturn {
        let spec = args.specifier;

        // Imports inside a require facade stay external at their CDN URL — the
        // facade's `import * as m from ...` must survive into the output.
        if args
            .importer
            .is_some_and(|i| i.starts_with(EXTERNAL_REQUIRE_FACADE))
        {
            return Ok(Some(HookResolveIdOutput {
                id: spec.to_string().into(),
                external: Some(ResolvedExternal::Bool(true)),
                ..Default::default()
            }));
        }

        // Package-internal module: the id is VIRTUAL_PREFIX + the storage key.
        // The entry input arrives in this form.
        if let Some(rest) = spec.strip_prefix(VIRTUAL_PREFIX) {
            let name = rest
                .strip_prefix(&self.cache_base)
                .and_then(|r| r.strip_prefix('/'))
                .unwrap_or_default();
            if let Some(cand) = resolve_in_files(&self.files, name) {
                return Ok(Some(HookResolveIdOutput::from_id(format!(
                    "{VIRTUAL_PREFIX}/{}/{cand}",
                    self.cache_base
                ))));
            }
            return Ok(None);
        }

        // Node subpath imports (`#foo`), mapped through package.json "imports".
        // Unmapped `#` specifiers never resolve (let-else keeps them away from the
        // package-name branch below, which would build a broken URL).
        if spec.starts_with('#') {
            let cand = self
                .imports
                .get(spec)
                .and_then(|target| resolve_in_files(&self.files, target));
            if let Some(cand) = cand {
                return Ok(Some(HookResolveIdOutput::from_id(format!(
                    "{VIRTUAL_PREFIX}/{}/{cand}",
                    self.cache_base
                ))));
            }
            return Ok(None);
        }

        // Relative import within the package.
        if spec.starts_with('.') {
            if let Some(id) = args
                .importer
                .and_then(|importer| self.resolve_relative(importer, spec))
            {
                return Ok(Some(HookResolveIdOutput::from_id(id)));
            }
            return Ok(None);
        }

        // Bare specifier: an npm dependency (or a node builtin / absolute path /
        // URL scheme, which never map to our CDN and stay external as-is).
        let looks_like_package = !spec.starts_with('/')
            && !spec.starts_with('\\')
            && !spec.contains(':')
            && !spec.contains('\\')
            && !is_node_builtin(spec);
        if !looks_like_package {
            return Ok(None);
        }

        let (pkg, subpath) = split_bare_specifier(spec);
        let url = self.cdn_url(pkg, subpath);
        if matches!(args.kind, ImportKind::Require) {
            return Ok(Some(HookResolveIdOutput::from_id(format!(
                "{EXTERNAL_REQUIRE_FACADE}{url}"
            ))));
        }
        Ok(Some(HookResolveIdOutput {
            id: url.into(),
            external: Some(ResolvedExternal::Bool(true)),
            ..Default::default()
        }))
    }

    async fn load(
        &self,
        _ctx: rolldown::plugin::SharedLoadPluginContext,
        args: &HookLoadArgs<'_>,
    ) -> rolldown::plugin::HookLoadReturn {
        // CJS require of an external → virtual module that re-exports the
        // external's ESM namespace. When the dependency itself is a converted CJS
        // package its exports live on `default`; a plain ESM dependency falls
        // through to the namespace (mirrors Node's require(esm) semantics).
        if let Some(url) = args.id.strip_prefix(EXTERNAL_REQUIRE_FACADE) {
            let code = format!(
                "import * as m from '{url}';module.exports = \
                 Object.prototype.hasOwnProperty.call(m, 'module.exports') \
                 ? m['module.exports'] : (m.default ?? m);"
            );
            return Ok(Some(HookLoadOutput {
                code: code.into(),
                ..Default::default()
            }));
        }

        if let Some(key) = args.id.strip_prefix(VIRTUAL_PREFIX)
            && let Some(data) = self.storage.get_raw(key).await
        {
            let code = String::from_utf8_lossy(&data);
            return Ok(Some(HookLoadOutput {
                code: code.into_owned().into(),
                ..Default::default()
            }));
        }
        Ok(None)
    }

    fn register_hook_usage(&self) -> HookUsage {
        HookUsage::ResolveId | HookUsage::Load
    }
}

/// `@scope/name[/subpath]` or `name[/subpath]` → (package, subpath).
fn split_bare_specifier(spec: &str) -> (&str, &str) {
    if let Some(rest) = spec.strip_prefix('@')
        && let Some((scope, remainder)) = rest.split_once('/')
    {
        return match remainder.split_once('/') {
            Some((name, sub)) => (&spec[..scope.len() + 1 + name.len() + 1], sub),
            None => (spec, ""),
        };
    }
    match spec.split_once('/') {
        Some((name, sub)) => (name, sub),
        None => (spec, ""),
    }
}

fn is_node_builtin(spec: &str) -> bool {
    if spec.starts_with("node:") {
        return true;
    }
    let root = spec.split('/').next().unwrap_or(spec);
    NODE_BUILTINS.contains(&root)
}

/// Newest published version of `package_name` satisfying `req`, or `None` if
/// metadata is unavailable or no version matches. Returning `None` (rather than
/// erroring) keeps bundling resilient to a registry hiccup — the import URL just
/// omits the version and the CDN resolves it per-request.
async fn latest_version_satisfying(
    storage: &SharedStorage,
    package_name: &str,
    req: &node_semver::Range,
) -> Option<String> {
    let metadata = crate::cdn::registry::fetch_npm_metadata(storage, package_name)
        .await
        .ok()?;
    let versions = metadata.get("versions")?.as_object()?;
    versions
        .keys()
        .filter_map(|s| s.parse::<node_semver::Version>().ok())
        .filter(|v| v.satisfies(req))
        .max()
        .map(|v| v.to_string())
}
