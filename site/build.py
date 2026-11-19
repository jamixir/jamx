#!/usr/bin/env python3
"""Build the self-hosted site from page.html.

page.html is the page without a document skeleton (title, styles, content, script).
This script wraps it in one and writes index.html, which can be served from any static
host.

    python3 build.py                              # waitlist not connected
    python3 build.py --endpoint https://...       # waitlist posts JSON to that URL
    python3 build.py --url https://example.org/   # canonical address, for link previews

The waitlist endpoint must accept a JSON POST of {email, role, note, at} and answer 2xx.
"""
import argparse
import html
import pathlib
import re

HERE = pathlib.Path(__file__).parent
DESCRIPTION = (
    "JAMX is an exchange with no vault: orders match on JAM, bitcoin settles on Bitcoin, "
    "and no key can move more than one person's funds. A working prototype."
)
RESET = (
    ":root{color-scheme:light;padding:env(safe-area-inset-top,0px) 0 env(safe-area-inset-bottom,0px)}"
    "body{margin:0;font:14px system-ui,sans-serif}img{max-width:100%}[hidden]{display:none!important}"
)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--endpoint", default="")
    ap.add_argument("--url", default="")
    args = ap.parse_args()

    page = (HERE / "page.html").read_text()
    title = re.search(r"<title>(.*?)</title>", page).group(1)
    body = re.sub(r"<title>.*?</title>\n?", "", page, count=1)
    body = re.sub(r'<meta name="claude:[^>]*>\n?', "", body)

    marker = 'var WAITLIST_ENDPOINT = "";'
    assert body.count(marker) == 1
    body = body.replace(marker, "var WAITLIST_ENDPOINT = %s;" % repr(args.endpoint).replace("'", '"'))

    head = [
        '<meta charset="utf-8">',
        '<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">',
        "<title>%s: an exchange with no vault</title>" % html.escape(title),
        '<meta name="description" content="%s">' % html.escape(DESCRIPTION),
        '<meta property="og:type" content="website">',
        '<meta property="og:title" content="JAMX: an exchange with no vault">',
        '<meta property="og:description" content="%s">' % html.escape(DESCRIPTION),
        '<meta name="twitter:card" content="summary">',
        '<link rel="preconnect" href="https://fonts.googleapis.com">',
        '<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>',
        "<style>%s</style>" % RESET,
    ]
    if args.url:
        head.insert(4, '<link rel="canonical" href="%s">' % html.escape(args.url))
        head.insert(5, '<meta property="og:url" content="%s">' % html.escape(args.url))

    out = '<!doctype html>\n<html lang="en">\n<head>\n%s\n</head>\n<body>\n%s</body>\n</html>\n' % ("\n".join(head), body)
    (HERE / "index.html").write_text(out)
    print("wrote", HERE / "index.html", len(out), "bytes")


if __name__ == "__main__":
    main()
