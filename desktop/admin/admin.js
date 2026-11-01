// The admin console's shell (admin console spec, Web UI). It shows the
// appliance's status, and the sign-in until there is a session; each
// section arrives with its own part of the console.
"use strict";

const $ = (s, el = document) => el.querySelector(s);

async function status() {
  const el = $("#status");
  try {
    const r = await fetch("/api/v1/health", { credentials: "same-origin" });
    const v = await r.json();
    el.dataset.state = v.ok ? "ok" : "down";
    $(".small", el).textContent = v.ok ? `Running · ${v.version}` : "Not answering";
  } catch {
    el.dataset.state = "down";
    $(".small", el).textContent = "Not answering";
  }
}

addEventListener("DOMContentLoaded", () => {
  status();
  setInterval(status, 15000);
});
