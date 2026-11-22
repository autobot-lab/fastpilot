// fastpilot ui, not signed in (E3): a browser the person chose to remember keeps a key in this
// page's localStorage and trades it for a session; anything else says how to sign in. The key
// never leaves this page except to /api/unlock on the same address.
"use strict";
(async () => {
  const KEY = "fastpilot.browser";
  const out = () => {
    document.getElementById("busy").hidden = true;
    document.getElementById("out").hidden = false;
    document.getElementById("out-t").focus?.();
  };
  const key = localStorage.getItem(KEY);
  if (!key) return out();
  try {
    const r = await fetch("/api/unlock", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ key }) });
    if (r.ok) {
      // A reload, not location.replace(): the same address with only a #… part would not load.
      location.reload();
      return;
    }
    if (r.status === 401) localStorage.removeItem(KEY);
  } catch (_) {
    // The dashboard stopped answering: say how to sign in.
  }
  out();
})();
