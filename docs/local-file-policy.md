# Local file loading and hosted browser policy

Moli can open a regular file from a browser navigation. This includes the URL
passed to `moli fetch` and an explicit CDP or WebDriver navigation. A Document
created by the file loader can follow local links and load relative resources
such as classic scripts, stylesheets, images and fonts. Synthetic child
Documents inherit their creator's authority with the creator's environment.
Sandbox policy is checked when the Document commits.

The startup configuration controls whether the browser grants file access.

| Flag | Environment variable | Effect |
| --- | --- | --- |
| `--block-file-urls` | `MOLI_BLOCK_FILE_URLS=1` | Reject file URL navigation and resource loads. |
| `--block-host-file-access` | `MOLI_BLOCK_HOST_FILE_ACCESS=1` | Also reject caller-selected host paths for file inputs, file drags and download destinations. |

The strongest configured restriction applies throughout the browser instance.
Both variables accept boolean values. An invalid value fails startup. Web
security switches and automation commands operate within the startup policy.

## Local file authority

Moli routes file URLs to a dedicated loader. The loader admits an authorized
direct navigation or a request from the exact active Document and checks request
mode before opening a file on a blocking executor. A navigation seeds file
authority when its response came from the file loader. The authority stays
with the document lifetime across subsequent resource requests.

This follows Chromium's division between navigation admission, document
authority and response visibility. File documents use opaque origins in the
default mode. A local image can be displayed while its canvas pixels remain
unavailable to script. A local stylesheet can apply while its rules remain
unavailable through CSSOM. HTTP and HTTP redirects use the HTTP transport's
separate scheme restrictions. [Chromium file loader](https://chromium.googlesource.com/chromium/src/+/main/content/browser/loader/file_url_loader_factory.cc)
[Chromium document loader selection](https://chromium.googlesource.com/chromium/src/+/main/content/browser/renderer_host/render_frame_host_impl.cc)

## Hosted deployment

For a service that accepts arbitrary URLs, enable file URL blocking and apply
the service's network egress policy.

```sh
MOLI_BLOCK_PRIVATE_NETWORKS=1 MOLI_BLOCK_FILE_URLS=1 moli serve
```

For a service that exposes CDP or WebDriver to tenants, enable host file access
blocking. Classic WebDriver can still attach a file uploaded through its own
session and download into its server-created Selenium download directory.

```sh
MOLI_BLOCK_PRIVATE_NETWORKS=1 MOLI_BLOCK_HOST_FILE_ACCESS=1 moli serve
```

Keep browser processes under a non-root identity with minimal mounts and
credentials. Use per-tenant process or container isolation, private artifact
directories and an authenticated control plane. Moli enforces file policy at
browser API boundaries. OS access after native execution inherits the process's
permissions. Filter proxy-resolved destinations at the proxy and enforce egress
restrictions at the deployment boundary.
