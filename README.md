# Nexus

![GitHub](https://img.shields.io/github/license/funish/nexus)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](https://www.contributor-covenant.org/version/2/1/code_of_conduct/)

> Universal package registry and CDN - WinGet registry mirror, npm, GitHub, JSR, cdnjs, WordPress, and 40+ package registries.

## Features

- 🪟 **WinGet Registry** - Complete Windows Package Manager mirror with search, package metadata, and version information
- 📦 **Universal CDN** - Access packages from npm, GitHub, JSR, cdnjs, WordPress, and more
- 🔒 **SRI Support** - Built-in Subresource Integrity (SHA-256) for secure CDN resource loading
- 🔄 **Registry Mirror** - Proxy requests to 40+ package registries (PyPI, crates.io, Go, Maven, Docker, etc.)
- 📚 **OpenAPI Documentation** - Interactive API docs available at `/_docs/scalar`, `/_docs/swagger`, and `/_docs/openapi.json`

## Usage

### CDN Endpoints

#### npm Packages

Access any npm package and its files:

```bash
# Get package entry file
curl https://nexus.funish.net/cdn/npm/react

# Get specific version
curl https://nexus.funish.net/cdn/npm/react@18

# Get specific file
curl https://nexus.funish.net/cdn/npm/react@18/index.js

# Get package metadata
curl https://nexus.funish.net/cdn/npm/react@18/package.json

# Get bundled ESM module
curl https://nexus.funish.net/cdn/npm/d3@7/+esm
```

**Version resolution:**

- `react` → latest version
- `react@18` → latest 18.x.x version
- `react@18.3` → latest 18.3.x version
- `react@18.3.1` → exact version

**ESM Bundling (`+esm`):**

The `+esm` endpoint bundles npm packages into a single ESM module with external dependencies resolved to CDN paths:

```html
<script type="module">
  import { scaleLinear } from 'https://nexus.funish.net/cdn/npm/d3@7/+esm';
</script>
```

External dependencies are automatically converted to CDN URLs (e.g., `d3-array` → `/cdn/npm/d3-array@3/+esm`).

#### GitHub Releases

Access files from GitHub repository releases:

```bash
# Get repository files (uses main branch by default)
curl https://nexus.funish.net/cdn/gh/vuejs/core

# Get specific tag/version
curl https://nexus.funish.net/cdn/gh/vuejs/core@v3.4.0

# Get specific file
curl https://nexus.funish.net/cdn/gh/vuejs/core@v3.4.0/package.json
```

#### JSR (JavaScript Registry)

Access JSR packages for Deno and Node.js:

```bash
# Get package entry file
curl https://nexus.funish.net/cdn/jsr/@std/path

# Get specific version
curl https://nexus.funish.net/cdn/jsr/@std/path@1.1.4

# Get specific file
curl https://nexus.funish.net/cdn/jsr/@std/path@1.1.4/mod.ts
```

#### cdnjs Libraries

Access libraries from cdnjs:

```bash
# @ format (recommended)
curl https://nexus.funish.net/cdn/cdnjs/jquery@3.7.1/jquery.min.js

# Original format
curl https://nexus.funish.net/cdn/cdnjs/jquery/3.7.1/jquery.min.js

# List all files
curl https://nexus.funish.net/cdn/cdnjs/jquery@3.7.1/
```

#### WordPress Plugins & Themes

Access WordPress resources from SVN:

```bash
# Plugin from tags
curl https://nexus.funish.net/cdn/wp/plugins/wp-slimstat/tags/4.6.5/wp-slimstat.js

# Plugin from trunk (latest development)
curl https://nexus.funish.net/cdn/wp/plugins/wp-slimstat/trunk/wp-slimstat.js

# Theme
curl https://nexus.funish.net/cdn/wp/themes/twentytwentyfour/1.0/style.css
```

### Subresource Integrity (SRI)

All CDN endpoints (npm, GitHub, JSR) include SHA-256 integrity hashes for secure resource loading. The browser will verify that the file hasn't been tampered with during delivery.

#### Get Integrity Hashes

```bash
# Get package listing with integrity hashes
curl https://nexus.funish.net/cdn/npm/react@18.3.1/

# Response includes integrity for each file:
{
  "name": "react",
  "version": "18.3.1",
  "files": [
    {
      "name": "index.js",
      "size": 12345,
      "integrity": "sha256-ABC123..."
    }
  ]
}
```

#### Use SRI in HTML

```html
<!-- npm package -->
<script src="https://nexus.funish.net/cdn/npm/react@18.3.1/index.js"
        integrity="sha256-ABC123..."
        crossorigin="anonymous"></script>

<!-- GitHub release -->
<script src="https://nexus.funish.net/cdn/gh/vuejs/core@v3.4.0/dist/vue.global.js"
        integrity="sha256-XYZ789..."
        crossorigin="anonymous"></script>

<!-- JSR package -->
<script type="module"
        src="https://nexus.funish.net/cdn/jsr/@std/path@1.0.0/mod.ts"
        integrity="sha256-DEF456..."
        crossorigin="anonymous"></script>
```

**Important Notes:**

- ✅ **Use exact versions** (e.g., `react@18.3.1`) - SRI is only provided for complete versions
- ✅ **Include `crossorigin="anonymous"`** - Required for cross-origin SRI verification
- ❌ **Avoid version ranges** (e.g., `react@18`) - Hash may change as versions update

### Registry Mirror

Proxy requests to any supported package registry by replacing the registry URL with `https://nexus.funish.net/mirror/{registry}`.

#### Supported Registries

**JavaScript/TypeScript:**

- `npm` - npm registry
- `jsr` - JSR registry

**Python:**

- `pypi` - PyPI

**Rust:**

- `crates` - crates.io

**Go:**

- `go` - Go Proxy

**Java:**

- `maven` - Maven Central
- `gradle` - Gradle Plugin Portal

**PHP:**

- `composer` - Packagist

**Docker:**

- `docker` - Docker Hub
- `ghcr` - GitHub Container Registry
- `quay` - Quay.io

**Flutter/Dart:**

- `pub` - pub.dev

**Package Managers:**

- `homebrew` - Homebrew Formulae
- `chocolatey` - Chocolatey
- `conda` / `condaforge` - Conda
- `nix` - Nix packages
- `guix` - Guix packages

**Linux Distributions:**

- `debian` / `ubuntu` - Debian/Ubuntu repositories
- `fedora` / `epel` - Fedora/EPEL repositories
- `arch` - Arch Linux
- `alpine` - Alpine Linux
- `gentoo` - Gentoo
- `openwrt` - OpenWrt

#### Configuration Examples

**npm**

```bash
# .npmrc
registry=https://nexus.funish.net/mirror/npm/
```

**pip (PyPI)**

```bash
# pip.conf
[global]
index-url = https://nexus.funish.net/mirror/pypi/simple
```

**cargo (crates.io)**

```toml
# .cargo/config.toml
[source.nexus]
registry = "https://nexus.funish.net/mirror/crates/"
[source.crates-io]
replace-with = "nexus"
```

**Go**

```bash
# go.env
GOPROXY=https://nexus.funish.net/mirror/go,https://proxy.golang.org,direct
```

**Docker**

```json
// daemon.json
{
  "registry-mirrors": [
    "https://nexus.funish.net/mirror/docker"
  ]
}
```

**Homebrew**

```bash
# Replace default bottle URLs
export HOMEBREW_BOTTLE_DOMAIN=https://nexus.funish.net/mirror/homebrew
```

**Yarn (npm)**

```bash
yarn config set registry https://nexus.funish.net/mirror/npm/
```

**pnpm (npm)**

```bash
pnpm config set registry https://nexus.funish.net/mirror/npm/
```

**Maven**

```xml
<!-- pom.xml or settings.xml -->
<repositories>
  <repository>
    <id>nexus</id>
    <url>https://nexus.funish.net/mirror/maven/</url>
  </repository>
</repositories>
```

**Gradle**

```gradle
repositories {
  maven { url 'https://nexus.funish.net/mirror/gradle/' }
}
```

### WinGet Registry

Complete Windows Package Manager registry mirror, compliant with [WinGet RESTSource API 1.9.0](https://github.com/microsoft/winget-cli-restsource/blob/main/documentation/WinGet-1.9.0.yaml) specification.

**Available endpoints:**

```bash
# List all packages
GET /registry/winget/packages

# Get package details
GET /registry/winget/packages/{id}

# Search packages
GET /registry/winget/manifestSearch?query={query}&matchType={CaseInsensitive|Exact|Fuzzy}

# Get package versions
GET /registry/winget/packages/{id}/versions

# Get specific version
GET /registry/winget/packages/{id}/versions/{version}

# Get all locales for a version
GET /registry/winget/packages/{id}/versions/{version}/locales

# Get specific locale
GET /registry/winget/packages/{id}/versions/{version}/locales/{locale}

# Get all installers for a version
GET /registry/winget/packages/{id}/versions/{version}/installers

# Get specific installer
GET /registry/winget/packages/{id}/versions/{version}/installers/{installer}
```

**Example:**

```bash
curl https://nexus.funish.net/registry/winget/packages/Microsoft.VisualStudioCode/versions
```

### API Documentation

Interactive API documentation is available:

- **Scalar UI**: https://nexus.funish.net/_docs/scalar
- **Swagger UI**: https://nexus.funish.net/_docs/swagger
- **OpenAPI Spec**: https://nexus.funish.net/_docs/openapi.json

## Support & Community

- 📫 [Report Issues](https://github.com/funish/axis/issues)

## License

This project is licensed under the MIT License - see the [LICENSE](./LICENSE) file for details.

---

Built with ❤️ by [Funish](http://www.funish.net/)
