# JAMX site

A single static page and one function for the waitlist.

| File | What it is |
|---|---|
| `page.html` | The source: title, styles, content and script, without a document skeleton. Edit this one. |
| `build.py` | Wraps `page.html` into `index.html`. |
| `index.html` | The built page. Committed, so the host needs no build step. |
| `api/waitlist.js` | Stores each sign-up as a private JSON file in Vercel Blob. |

## Deploying on Vercel

1. Import the repository and set **Root Directory** to `site`. Framework preset: Other. No
   build command.
2. In the project's **Storage** tab, create a Blob store with **private** access and connect
   it to the project. Redeploy.
3. Sign-ups appear in that store under `waitlist/`.

Until a store is connected the form says that the sign-up was not saved. It never reports
a success that did not happen.

## Rebuilding

```sh
python3 build.py --endpoint /api/waitlist --url https://your-domain/
```

`--endpoint` is where the form posts `{email, role, note, at}` as JSON; any service that
answers 2xx will do. Without it the form is not connected.
