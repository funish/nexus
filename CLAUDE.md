# Nexus — High-Performance ESM CDN & Package Proxy

Nexus is a Rust-based CDN service that proxies npm/JSR/GitHub packages, converts them to browser-native ESM, and caches the results. It also provides a WinGet package search API backed by SQLite.

## Tech Stack

| Layer | Technology | Purpose |
|-------|-----------|---------|
| HTTP | **axum** + **tokio** | Async web server |
| ESM bundling | **rolldown** | CJS→ESM conversion, bundling, minification |
| JS minify | **oxc** | Single-file JS/TS minification (same toolchain as rolldown, pinned 0.135) |
| CSS minify | **lightningcss** | CSS minification |
| SQLite | **rusqlite** | WinGet index.db queries |
| Storage | **tokio::fs** + **aws-sdk-s3** | Local cache + S3 for distributed deployments |
| Tar | **tar** + **flate2** | .tgz extraction |
| Hash | **sha2** | SHA-256 integrity (SRI) |
| HTTP client | **reqwest** | npm registry + tarball fetching |
| Serialization | **serde** + **serde_json** | JSON handling |
| Search | **strsim** | Fuzzy string matching for WinGet |
| MIME | **mime_guess** | Content-Type detection |
| Semver | **node-semver** + **semver** | node-semver: npm-compatible CDN version resolution; semver: VersionReq range parsing + version comparison |

## Project Structure

```
src/
  main.rs                 # axum server entry; registers cdn + winget routers
  config.rs               # Environment variable configuration
  error.rs                # Unified error handling (thiserror AppError)
  cdn/                    # /cdn/** — behavior aligned with jsDelivr
    mod.rs                # pub fn router()
    routes/               # Route handlers (one file per route)
      npm.rs              # /cdn/npm/* (entry, listing, +esm, sub-path, org listing)
      jsr.rs              # /cdn/jsr/*
      gh.rs               # /cdn/gh/*
      cdnjs.rs            # /cdn/cdnjs/*
      wp.rs               # /cdn/wp/* (WordPress plugins/themes)
      combine.rs          # /cdn/combine/* (concatenate npm/gh files)
    utils/                # Route-agnostic CDN logic
      registry.rs         # npm/jsr/cdnjs/gh/org metadata fetching (TTL-cached)
      resolve.rs          # Version resolution (node-semver, dist-tags, gh tags)
      tarball.rs          # .tgz download, extraction, file/package caching
      esm.rs              # ESM bundling via rolldown
      entry.rs            # package.json entry resolution (default file, +esm priority)
      minify.rs           # JS (oxc) / CSS (lightningcss) minification, .min synthesis
      integrity.rs        # SHA-256 for Subresource Integrity
      cache.rs            # TTL cache helpers (mtime expiry, cached_json)
      singleflight.rs     # Dedup concurrent cache-miss requests (run_once)
      listing.rs          # Directory listing JSON generation
      mime.rs             # Extension → MIME type mapping
      constants.rs        # Cache tiers + size limits
  winget/                 # /api/winget/** — WinGet RESTSource spec
    mod.rs                # pub fn router()
    routes/               # WinGet REST handlers
      catalog.rs          # manifestSearch, packages, information, package details
      manifests.rs        # versions, installers, locales, packageManifests
    utils/                # WinGet logic
      db.rs               # SQLite index.db (rusqlite) + persisted search index
      search.rs           # Fuzzy search (strsim)
      queries.rs          # SQL queries for package/version lookup
      manifest.rs         # Manifest resolution from GitHub tree
      tree.rs             # GitHub tree SHA traversal
      token.rs            # Continuation token encode/decode
      response.rs         # WinGet REST response types + helpers
      http.rs             # Shared reqwest client (connection pool)
      constants.rs        # Manifest URLs, limits
  storage/
    mod.rs                # Storage trait definition + CacheMeta
    fs.rs                 # Filesystem storage (tokio::fs)
    s3.rs                 # S3 storage (aws-sdk-s3)
```

## Build & Run

```bash
cargo build              # Debug build
cargo build --release    # Release build
cargo run                # Run debug build
cargo test               # Run tests
cargo clippy             # Lint
cargo fmt                # Format code
```

Docker:
```bash
docker build -t nexus .
docker run -p 3000:3000 nexus
```

## Architecture

### CDN Proxy Flow

```
Client → /cdn/npm/PACKAGE@VERSION/+esm
  1. Resolve version from npm registry (semver range → exact version)
  2. Check S3/fs cache for pre-built bundle
  3. Cache miss: download tarball → extract → rolldown bundle → cache result
  4. Rewrite external imports to /cdn/npm/dep@version/+esm paths
  5. Return ESM bundle with immutable Cache-Control
```

### ESM Bundling (esm.rs)

Uses rolldown's Rust API to bundle npm packages:

1. Extract tarball to temp directory
2. Read package.json → resolve dependencies
3. Configure rolldown: `format: ESM`, `platform: Browser`, `external: deps`
4. Run `bundler.write()` → get bundled output
5. Rewrite bare imports to CDN paths (`/cdn/npm/dep@version/+esm`)
6. Cache result in storage
7. Clean up temp directory

### Storage Layer

Trait-based abstraction supporting multiple backends:

```rust
#[async_trait]
trait Storage {
    async fn get_raw(&self, key: &str) -> Option<Vec<u8>>;
    async fn set_raw(&self, key: &str, data: &[u8]);
    async fn get_meta(&self, key: &str) -> Option<Meta>;
    async fn set_meta(&self, key: &str, meta: &Meta);
}
```

- **Development**: Filesystem (`.cache/` directory)
- **Production**: S3 (if `S3_ACCESS_KEY_ID` etc. are set), otherwise filesystem

### jsDelivr alignment

`/cdn/**` mirrors jsDelivr behavior:
- **Default file**: `jsdelivr` > `browser` > `main` (JS), `style` (CSS); always served minified.
- **`.min` synthesis**: requesting `foo.min.js` when only `foo.js` exists returns minified output (cached for reuse).
- **Version fallback**: when the newest version matching a range lacks a file, older matching versions are tried (up to 2).
- **Cache tiers**: exact version/commit → 1yr immutable; range/latest tag → 7d; branch → 12h.
- **HTML safety**: `.html`/`.htm` served as `text/plain`.
- **Single-flight**: concurrent cache-miss requests for the same key share one download/bundle (`cdn::utils::singleflight::run_once`).

### WinGet Search

Downloads `source.msix` from Microsoft, extracts `index.db`, loads into SQLite.
Provides fuzzy search over package names, publishers, tags, and commands.

## Route Design

```
GET /cdn/npm/:package              → Entry file (proxy)
GET /cdn/npm/:package/             → Directory listing JSON
GET /cdn/npm/:package/+esm         → ESM bundle
GET /cdn/npm/:package/*path        → Sub-path file
GET /cdn/npm/:package@version      → Specific version entry
GET /cdn/npm/:package@version/+esm → Specific version ESM bundle
GET /cdn/npm/:package@version/*    → Specific version sub-path
GET /cdn/npm/@scope/:package...    → Scoped packages (same patterns)
GET /cdn/combine/:paths            → Concatenate files (comma-separated npm/gh paths)

GET /api/winget/packages           → Search packages
GET /api/winget/packages/:id       → Package details
GET /api/winget/packages/:id/versions → Package versions
```

## Naming Conventions

- **Functions**: `snake_case` (Rust convention)
- **Files**: `snake_case.rs`
- **Types**: `PascalCase`
- **Constants**: `SCREAMING_SNAKE_CASE`
- **Modules**: one concern per file, re-export from `mod.rs`

## Behavioral Guidelines

- State assumptions explicitly. If uncertain, ask before implementing.
- No features beyond what was asked. No speculative abstractions.
- Touch only what you must. Match existing style.
- Prefer `&str` over `String` where possible. Use `Cow<str>` for conditional ownership.
- Use `thiserror` for library errors, `anyhow` for application errors.
- All async code uses tokio runtime. No blocking calls in async context.
- Minimize allocations in hot paths (request handling, tar parsing).
