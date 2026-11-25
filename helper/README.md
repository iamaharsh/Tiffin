# tiffin-helper

The one server behind Tiffin on the web. A browser can't read other websites' pages (CORS), so the web app asks this helper to fetch them. Your recipes and food diary never come here: they stay in your browser.

## What it does

| Route | Does | Sends out |
|---|---|---|
| `GET /page?url=…` | Fetches a recipe page and returns its HTML, labelled as plain text so no browser runs it | The page address |
| `GET /photo?url=…` | Fetches a recipe picture | The picture address |
| `GET /off?q=…&email=…` | Searches Open Food Facts by food name | The search text, with your contact email in the User-Agent, as Open Food Facts asks |

Barcode lookups don't come here: the browser asks Open Food Facts for those directly.

It keeps nothing: no database, no saved files, and no logs of addresses, searches or emails. Every link goes through the same rules as the desktop app: http and https only, never this computer or a private network (every redirect is checked again), at most 10 MB for a page and 15 MB for a picture, at most 5 redirects and 50 seconds.

It answers only the Tiffin web app (its exact `Origin`). Each visitor gets 8 downloads and 8 searches a minute, and at most 16 requests run at once. When Open Food Facts says to wait, every search waits.

## Run it on your own server

Build the image from the repository root of a **release snapshot**. The dev tree's full exercise (MET) table may not be redistributed yet, and the snapshot carries the stand-in.

```sh
docker build -f helper/Dockerfile -t tiffin-helper .
docker run -d --name tiffin-helper --restart unless-stopped --read-only -p 127.0.0.1:8787:8787 \
  -e TIFFIN_ORIGIN=https://tiffin.fyi -e TIFFIN_BEHIND_PROXY=1 tiffin-helper
```

Set `TIFFIN_ORIGIN` to your own web app's address if it isn't tiffin.fyi.

Or without Docker: `cargo build --release --locked --manifest-path helper/Cargo.toml` (on Linux, install OpenSSL's development files first). If that machine has curl, the importer may use it as a second try, writing a temporary file it deletes straight after.

### Settings

| Variable | Default | Meaning |
|---|---|---|
| `TIFFIN_HELPER_ADDR` | `0.0.0.0:8787` | Where it listens |
| `TIFFIN_ORIGIN` | `https://tiffin.fyi` | The web app's address. Separate several with commas, e.g. `https://tiffin.fyi,http://localhost:5173` while developing |
| `TIFFIN_BEHIND_PROXY` | off | Set it to `1` only behind a reverse proxy that sets the visitor IP header. By default the helper counts limits for the last address in `X-Forwarded-For` |
| `TIFFIN_CLIENT_IP_HEADER` | *(unset)* | Optional single-IP header used when `TIFFIN_BEHIND_PROXY=1`, e.g. `fly-client-ip` on Fly.io (where `X-Forwarded-For` ends with the Fly Proxy IP instead of the visitor) |

### HTTPS

The helper speaks plain HTTP. Put it behind a reverse proxy for HTTPS, for example Caddy:

```
helper.tiffin.fyi {
    reverse_proxy 127.0.0.1:8787
}
```

Caddy sets `X-Forwarded-For` to the real visitor, so run the helper with `TIFFIN_BEHIND_PROXY=1`. **Never set that without a proxy in front:** visitors could then claim any address and dodge the limits. Publish the port on `127.0.0.1` only, as above, so the proxy is the only way in.
