# Nexus

![GitHub](https://img.shields.io/github/license/funish/nexus)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](https://www.contributor-covenant.org/version/2/1/code_of_conduct/)

> Universal package registry and CDN — WinGet registry, npm, GitHub, JSR, cdnjs, WordPress. Built in Rust for speed.

Nexus is a high-performance ESM CDN and package proxy written in Rust (axum + tokio). It proxies npm / JSR / GitHub / cdnjs / WordPress packages, converts them to browser-native ESM, and caches the results. It also serves a complete WinGet package search and manifest API backed by SQLite + the winget-pkgs GitHub source.

## Features

- ⚡ **Rust + axum** — async, low-overhead HTTP server on tokio
- 🪟 **WinGet REST API** — search, package metadata, versions, installers, locales, and full manifests (RESTSource 1.9.0 compatible)
- 📦 **Universal CDN** — npm, GitHub, JSR, cdnjs, WordPress
- 🔧 **ESM Bundling** — npm packages bundled to browser-native ESM via rolldown, with bare imports rewritten to CDN paths
- 🔒 **SRI Support** — SHA-256 Subresource Integrity for every cached file
- 💾 **Pluggable storage** — filesystem for dev, S3 for distributed deployments

## Usage

### CDN Endpoints

#### npm Packages

```bash
# Get package entry file
curl https://nexus.funish.net/cdn/npm/react

# Get specific version (supports ranges: 18, 18.3, ^18.3.1)
curl https://nexus.funish.net/cdn/npm/react@18

# Get specific file
curl https://nexus.funish.net/cdn/npm/react@18/index.js

# Directory listing (trailing slash)
curl https://nexus.funish.net/cdn/npm/react@18.3.1/

# Get bundled ESM module
curl https://nexus.funish.net/cdn/npm/d3@7/+esm
```

**Version resolution** (npm-flavored semver via `node-semver`): `react` → latest · `react@18` → latest 18.x · `react@18.3` → latest 18.3.x · `react@18.3.1` → exact.

**ESM Bundling (`+esm`):** bundles an npm package into a single ESM module with external dependencies rewritten to CDN URLs:

```html
<script type="module">
  import { scaleLinear } from "https://nexus.funish.net/cdn/npm/d3@7/+esm";
</script>
```

#### GitHub Repositories

```bash
# Repository files (latest tag by default)
curl https://nexus.funish.net/cdn/gh/vuejs/core

# Specific tag/version (supports ranges and v-prefix)
curl https://nexus.funish.net/cdn/gh/vuejs/core@v3.4.0/dist/vue.global.js
```

#### JSR (JavaScript Registry)

```bash
curl https://nexus.funish.net/cdn/jsr/@std/path@1.1.4/mod.ts
```

#### cdnjs Libraries

```bash
# @ format (recommended) — supports version ranges
curl https://nexus.funish.net/cdn/cdnjs/jquery@3.7/jquery.min.js

# List all files for a version
curl https://nexus.funish.net/cdn/cdnjs/jquery@3.7.1/
```

#### WordPress Plugins & Themes

```bash
# Plugin from a tagged version
curl https://nexus.funish.net/cdn/wp/plugins/wp-slimstat/tags/4.6.5/wp-slimstat.js

# Plugin from trunk (latest development)
curl https://nexus.funish.net/cdn/wp/plugins/wp-slimstat/trunk/wp-slimstat.js

# Theme
curl https://nexus.funish.net/cdn/wp/themes/twentytwentyfour/1.0/style.css
```

#### Combine Multiple Files

Concatenate several npm/gh files into a single response (comma-separated, each prefixed with `npm/` or `gh/`):

```bash
curl "https://nexus.funish.net/cdn/combine/npm/jquery@3/dist/jquery.min.js,gh/twbs/bootstrap@3/dist/js/bootstrap.min.js"
```

The combined result is cached long-term (immutable); the `Content-Type` is taken from the first file.

### Subresource Integrity (SRI)

Every cached file gets a SHA-256 integrity hash, exposed in directory listings for use as an ETag and for SRI:

```bash
curl https://nexus.funish.net/cdn/npm/react@18.3.1/
# {
#   "name": "react",
#   "version": "18.3.1",
#   "files": [
#     { "name": "index.js", "size": 12345, "integrity": "sha256-ABC123..." }
#   ]
# }
```

```html
<script
  src="https://nexus.funish.net/cdn/npm/react@18.3.1/index.js"
  integrity="sha256-ABC123..."
  crossorigin="anonymous"
></script>
```

Conditional requests are honored: send `If-None-Match: <integrity>` to get a `304 Not Modified`.

### WinGet API

A complete Windows Package Manager REST API, compatible with the [WinGet RESTSource 1.9.0](https://github.com/microsoft/winget-cli-restsource/blob/main/documentation/WinGet-1.9.0.yaml) specification. Package search is backed by the SQLite `index.db`; per-version manifests are assembled from the `microsoft/winget-pkgs` GitHub YAML files.

```bash
# Server information
GET https://nexus.funish.net/api/winget/information

# List all packages (paginated)
GET https://nexus.funish.net/api/winget/packages

# Search packages (GET or POST)
GET  https://nexus.funish.net/api/winget/manifestSearch?query=vscode&matchType=Fuzzy&maximumResults=5
POST https://nexus.funish.net/api/winget/manifestSearch        # body: {"Query":{"KeyWord":"chrome","MatchType":"Fuzzy"}}

# Package metadata & versions
GET https://nexus.funish.net/api/winget/packages/{id}
GET https://nexus.funish.net/api/winget/packages/{id}/versions

# Per-version manifest data
GET https://nexus.funish.net/api/winget/packages/{id}/versions/{version}/installers
GET https://nexus.funish.net/api/winget/packages/{id}/versions/{version}/installers/{installer}
GET https://nexus.funish.net/api/winget/packages/{id}/versions/{version}/locales
GET https://nexus.funish.net/api/winget/packages/{id}/versions/{version}/locales/{locale}

# Full merged manifest (supports ?Version, ?Channel, ?Market filters)
GET https://nexus.funish.net/api/winget/packageManifests/{id}
```

**Example:**

```bash
curl https://nexus.funish.net/api/winget/packages/Microsoft.VisualStudioCode/versions
```

## Deployment

### Prerequisites

- **Rust** (stable, 1.88+ for edition 2024 + let-chains)

### Build & Run

```bash
cargo build --release        # build the optimized binary
cargo run --release          # run it

# or during development
cargo run
```

The server listens on `0.0.0.0:3000` by default (override with `PORT`).

### Docker Deployment

#### Option 1: Docker Compose (recommended)

```bash
cp .env.example .env
# edit .env to configure GITHUB_TOKEN / S3 / etc.
docker compose up -d
docker compose logs -f app
```

#### Option 2: Docker Run

```bash
docker pull funish/nexus:latest
docker run -d \
  --name nexus \
  -p 3000:3000 \
  --env-file .env \
  --restart unless-stopped \
  funish/nexus:latest
```

#### Option 3: Build from Source

```bash
docker build -t nexus .
docker run -d --name nexus -p 3000:3000 --env-file .env nexus
```

## Configuration

All configuration is via environment variables (see [`.env.example`](./.env.example)):

| Variable | Default | Description |
|---|---|---|
| `PORT` | `3000` | HTTP listen port |
| `CACHE_DIR` | `./.cache` | Filesystem cache directory (used when S3 is not configured) |
| `GITHUB_TOKEN` | — | Optional; raises GitHub API rate limits for WinGet manifest/tree fetching |
| `S3_ACCESS_KEY_ID` | — | S3 access key (enables S3 storage when all S3_* are set) |
| `S3_SECRET_ACCESS_KEY` | — | S3 secret key |
| `S3_ENDPOINT` | — | S3 endpoint URL |
| `S3_REGION` | — | S3 region |
| `S3_BUCKET` | — | S3 bucket name |

## License

This project is licensed under the MIT License — see the [LICENSE](./LICENSE) file for details.

---

Built with ❤️ by [Funish](http://www.funish.net/)
