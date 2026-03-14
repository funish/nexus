# Nexus

![GitHub](https://img.shields.io/github/license/funish/nexus)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](https://www.contributor-covenant.org/version/2/1/code_of_conduct/)

> Universal package registry and CDN - WinGet registry, npm, GitHub, JSR, cdnjs, WordPress, and many more package registries.

## Features

- 🪟 **WinGet API** - Complete Windows Package Manager REST API with search, package metadata, and version information
- 📦 **Universal CDN** - Access packages from npm, GitHub, JSR, cdnjs, WordPress, and more
- 🔒 **SRI Support** - Built-in Subresource Integrity (SHA-256) for secure CDN resource loading
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
<script
  src="https://nexus.funish.net/cdn/npm/react@18.3.1/index.js"
  integrity="sha256-ABC123..."
  crossorigin="anonymous"
></script>

<!-- GitHub release -->
<script
  src="https://nexus.funish.net/cdn/gh/vuejs/core@v3.4.0/dist/vue.global.js"
  integrity="sha256-XYZ789..."
  crossorigin="anonymous"
></script>

<!-- JSR package -->
<script
  type="module"
  src="https://nexus.funish.net/cdn/jsr/@std/path@1.0.0/mod.ts"
  integrity="sha256-DEF456..."
  crossorigin="anonymous"
></script>
```

**Important Notes:**

- ✅ **Use exact versions** (e.g., `react@18.3.1`) - SRI is only provided for complete versions
- ✅ **Include `crossorigin="anonymous"`** - Required for cross-origin SRI verification
- ❌ **Avoid version ranges** (e.g., `react@18`) - Hash may change as versions update

### WinGet API

Complete Windows Package Manager REST API, compliant with [WinGet RESTSource API 1.9.0](https://github.com/microsoft/winget-cli-restsource/blob/main/documentation/WinGet-1.9.0.yaml) specification.

**Available endpoints:**

```bash
# List all packages
GET /api/winget/packages

# Get package details
GET /api/winget/packages/{id}

# Search packages
GET /api/winget/manifestSearch?query={query}&matchType={CaseInsensitive|Exact|Fuzzy}

# Get package versions
GET /api/winget/packages/{id}/versions

# Get specific version
GET /api/winget/packages/{id}/versions/{version}

# Get all locales for a version
GET /api/winget/packages/{id}/versions/{version}/locales

# Get specific locale
GET /api/winget/packages/{id}/versions/{version}/locales/{locale}

# Get all installers for a version
GET /api/winget/packages/{id}/versions/{version}/installers

# Get specific installer
GET /api/winget/packages/{id}/versions/{version}/installers/{installer}
```

**Example:**

```bash
curl https://nexus.funish.net/api/winget/packages/Microsoft.VisualStudioCode/versions
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
