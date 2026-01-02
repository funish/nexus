# Nexus

![GitHub](https://img.shields.io/github/license/funish/nexus)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](https://www.contributor-covenant.org/version/2/1/code_of_conduct/)

> Universal package registry and CDN - WinGet registry mirror, npm, GitHub, JSR, cdnjs, WordPress, and 40+ package registries.

## Features

- 🪟 **WinGet Registry** - Complete Windows Package Manager mirror with search, package metadata, and version information
- 📦 **Universal CDN** - Access packages from npm, GitHub, JSR, cdnjs, WordPress, and more
- 🔄 **Registry Mirror** - Proxy requests to 40+ package registries (PyPI, crates.io, Go, Maven, Docker, etc.)
- 📚 **OpenAPI Documentation** - Interactive API docs at `/_docs/scalar` and `/_docs/swagger`

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
```

**Version resolution:**

- `react` → latest version
- `react@18` → latest 18.x.x version
- `react@18.3` → latest 18.3.x version
- `react@18.3.1` → exact version

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

WinGet endpoint provides a complete mirror of the Windows Package Manager registry.

#### List All Packages

```bash
curl https://nexus.funish.net/registry/winget/packages
```

**Response:**

```json
{
  "Data": [
    {
      "PackageIdentifier": "Microsoft.VisualStudioCode",
      "Versions": ["1.95.0", "1.94.0", "..."]
    }
  ],
  "ContinuationToken": "..."
}
```

#### Get Package Details

```bash
curl https://nexus.funish.net/registry/winget/packages/Microsoft.VisualStudioCode
```

#### Search Packages

```bash
# Case-insensitive search
curl "https://nexus.funish.net/registry/winget/manifestSearch?query=Adobe&matchType=CaseInsensitive"

# Exact match
curl "https://nexus.funish.net/registry/winget/manifestSearch?query=Microsoft.VisualStudioCode&matchType=Exact"

# Fuzzy search
curl "https://nexus.funish.net/registry/winget/manifestSearch?query=VSCode&matchType=Fuzzy"
```

#### Get Package Versions

```bash
curl https://nexus.funish.net/registry/winget/packages/Microsoft.VisualStudioCode/versions
```

#### Get Specific Version

```bash
curl https://nexus.funish.net/registry/winget/packages/Microsoft.VisualStudioCode/versions/1.95.0
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
