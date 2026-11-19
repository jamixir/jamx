// Waitlist sign-ups for the hosted site. Each one is stored as a private JSON file in the
// Vercel Blob store connected to this project, under waitlist/. Read them in the Vercel
// dashboard (Storage, then the store's browser).
import { put } from "@vercel/blob";

const ROLES = new Set(["person", "issuer", "maker", "funder", "builder"]);

function json(status, body) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store" },
  });
}

export async function POST(request) {
  let body;
  try {
    const raw = await request.text();
    if (raw.length > 4000) return json(413, { error: "too_large" });
    body = JSON.parse(raw);
  } catch {
    return json(400, { error: "bad_request" });
  }
  if (!body || typeof body !== "object") return json(400, { error: "bad_request" });

  const email = String(body.email || "").trim().toLowerCase();
  if (email.length > 254 || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
    return json(400, { error: "bad_email" });
  }
  const at = new Date().toISOString();
  const entry = {
    email,
    role: ROLES.has(body.role) ? body.role : "person",
    note: String(body.note || "").slice(0, 600),
    at,
  };

  try {
    await put(`waitlist/${at.replace(/[:.]/g, "-")}.json`, JSON.stringify(entry), {
      access: "private",
      contentType: "application/json",
      addRandomSuffix: true,
    });
  } catch (err) {
    // Most likely no Blob store is connected to the project yet.
    console.error("waitlist: sign-up not stored:", err && err.message);
    return json(503, { error: "not_stored" });
  }
  return json(200, { ok: true });
}
