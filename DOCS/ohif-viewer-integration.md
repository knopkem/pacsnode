# OHIF viewer integration

`pacsnode` can host a pre-built OHIF viewer (or another static DICOMweb UI) through the optional `ohif-viewer` plugin.

## What this plugin does

When enabled, the plugin:

- serves static files from a configured directory
- redirects `/` to the configured viewer prefix
- redirects the bare viewer prefix (for example `/viewer`) to `/viewer/`
- falls back to the configured HTML shell for browser SPA navigation under the viewer prefix

The plugin is compiled in, but it is disabled by default.

## Prepare the viewer assets

Build or obtain an OHIF distribution and copy the generated static files onto the pacsnode host. For example:

```bash
mkdir -p /opt/pacsnode/viewer
cp -R /path/to/ohif-build/* /opt/pacsnode/viewer/
```

Make sure the directory contains `index.html` and the rest of the generated assets.

## Enable the plugin

Add the plugin ID to `[plugins].enabled` and configure the static asset location:

```toml
[plugins]
enabled = ["ohif-viewer"]

[plugins.ohif-viewer]
static_dir = "/opt/pacsnode/viewer"
route_prefix = "/viewer"
redirect_root = true
index_file = "index.html"
fallback_file = "index.html"
```

`route_prefix` must be an absolute path and cannot be `/`.

After restarting `pacsnode`, the viewer will be reachable at `http://<host>:8042/viewer/`.

Set `redirect_root = true` to make `http://<host>:8042/` redirect to the viewer, or `false` if another route/plugin should own `/`.

## Authentication note

If `basic-auth` is enabled at the same time, allow the viewer shell to load before it starts making authenticated API requests:

```toml
[plugins.basic-auth]
public_paths = ["/health", "/metrics", "/", "/viewer"]
```

The viewer shell can stay public while your DICOMweb routes remain protected.

## Operational notes

- The plugin validates `static_dir`, `index_file`, and `fallback_file` during startup.
- Missing browser navigation routes under the viewer prefix return the configured fallback HTML document.
- Missing asset requests such as JavaScript bundles still return `404 Not Found`.
