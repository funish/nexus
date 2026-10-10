# Nexus — High-Performance ESM CDN & Package Proxy

Nexus is a Rust-based CDN service that proxies npm/JSR/GitHub packages, converts them to browser-native ESM, and caches the results. It also provides a WinGet package search API backed by SQLite.

## Tech Stack

| Layer | Technology | Purpose |
|-------|-----------|---------|
| HTTP | **axum** + **tokio** | Async web server |
| ESM bundling | **rolldown** | CJS→ESM conversion, bundling, minification |
| JS minify | **oxc** | Single-file JS/TS minification (same toolchain as rolldown, pinned 0.151) |
| CSS minify | **lightningcss** | CSS minification |
| SQLite | **rusqlite** | WinGet index.db queries |
| Storage | **tokio::fs** + **rust-s3** | Local cache + S3-compatible (RustFS) for distributed deployments |
| Tar | **tar** + **flate2** (zlib-rs backend) | .tgz extraction |
| Hash | **sha2** | SHA-256 integrity (SRI) |
| HTTP client | **reqwest** | Shared client for registry, archive, raw content, and API fetching |
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
  routes/                 # URL-facing handlers; route registration stays in each router module
    mod.rs
    cdn/                  # /cdn/**
      mod.rs
      npm.rs              # /cdn/npm/* (entry, listing, +esm, sub-path, org listing)
      jsr.rs              # /cdn/jsr/*
      gh.rs               # /cdn/gh/*
      cdnjs.rs            # /cdn/cdnjs/*
      wp.rs               # /cdn/wp/* (WordPress plugins/themes)
      combine.rs          # /cdn/combine/* (concatenate npm/gh files)
    api/
      mod.rs
      winget/             # /api/winget/** — WinGet RESTSource handlers
        mod.rs
        catalog.rs        # manifestSearch, packages, information, package details
        manifests.rs      # versions, installers, locales, packageManifests
  cdn/                    # Route-agnostic CDN domain logic and cache tiers
    mod.rs
    registry.rs           # npm/jsr/cdnjs/gh/org metadata fetching (TTL-cached)
    resolve.rs            # Version resolution (node-semver, dist-tags, gh tags)
    tarball.rs            # .tgz download, extraction, file/package caching
    esm.rs                # ESM bundling via rolldown
    entry.rs              # package.json entry resolution (default file, +esm priority)
    minify.rs             # JS (oxc) / CSS (lightningcss) minification, .min synthesis
    integrity.rs          # SHA-256 for Subresource Integrity
    response.rs           # CDN response headers and conditional requests
    listing.rs            # Directory listing JSON generation
    mime.rs               # Extension → MIME type mapping
    constants.rs          # Cache tiers + size limits
  winget/                 # WinGet domain logic
    mod.rs
    db.rs                 # SQLite index.db (rusqlite) + persisted search index
    search.rs             # Fuzzy search (strsim)
    queries.rs            # SQL queries for package/version lookup
    manifest.rs           # Manifest resolution and merging
    tree.rs               # GitHub tree SHA traversal
    rest.rs               # WinGet REST request/response types + helpers
    token.rs              # Continuation token encode/decode
    constants.rs          # Manifest URLs, limits
  utils/                  # Cross-domain cache, HTTP, concurrency, and single-flight helpers
  storage/
    mod.rs                # Storage trait definition + CacheMeta
    fs.rs                 # Filesystem storage (tokio::fs)
    s3.rs                 # S3-compatible storage (rust-s3)
```

## Build & Run

```bash
cargo build              # Debug build
cargo build --release    # Release build (LTO=fat, codegen-units=1, panic=abort)
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
    async fn set_raw(&self, key: &str, data: &[u8]) -> anyhow::Result<()>;
    async fn get_meta(&self, key: &str) -> Option<CacheMeta>;
    async fn set_meta(&self, key: &str, meta: &CacheMeta) -> anyhow::Result<()>;
}
```

- **Development**: Filesystem (`.cache/` directory)
- **Production**: S3-compatible (RustFS) via `rust-s3` (if `S3_ACCESS_KEY_ID` etc. are set), otherwise filesystem. `rust-s3` reuses the shared reqwest/rustls/hyper-1.x HTTP stack rather than pulling in the AWS SDK's duplicate runtime.

### jsDelivr alignment

`/cdn/**` mirrors jsDelivr behavior:
- **Default file**: `jsdelivr` > `browser` > `main` (JS), `style` (CSS); always served minified.
- **`.min` synthesis**: requesting `foo.min.js` when only `foo.js` exists returns minified output (cached for reuse).
- **Version fallback**: when the newest version matching a range lacks a file, older matching versions are tried (up to 2).
- **Cache tiers**: exact version/commit → 1yr immutable; range/latest tag → 7d browser / 12h shared cache; branch → 12h.
- **HTML safety**: `.html`/`.htm` served as `text/plain`.
- **Single-flight**: concurrent cache-miss requests for the same key share one download/bundle (`utils::singleflight::run_once`).

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
GET /cdn/jsr/:package/*path        → JSR file (proxy)
GET /cdn/gh/:owner/:repo/*path     → Repository file (proxy)
GET /cdn/cdnjs/:library/*path      → cdnjs file (proxy)
GET /cdn/wp/:type/:name/*path      → WordPress asset (proxy)
GET /cdn/combine/:paths            → Concatenate files (comma-separated npm/gh paths)

GET /api/winget/packages           → Search packages
GET /api/winget/information        → Server/source information
GET|POST /api/winget/manifestSearch → Search manifests
GET /api/winget/packages/:id       → Package details
GET /api/winget/packages/:id/versions → Package versions
GET .../installers, .../locales    → Per-version manifest slices
GET /api/winget/packageManifests/:id → Full merged manifest
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
