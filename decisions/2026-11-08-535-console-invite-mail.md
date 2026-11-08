# Console invites email again (2026-11-08, #535)

- The emailed invite is a real link: a kernel-served `/invite` page (form behind a live token, mirroring `/reset`), since no app screen consumes raw tokens. POST /invite joins the cookie-guard exemption: the emailed token is the authorization.
- `POST /v1/admin/mail` on the control socket is the console's generic send (to/subject/text → `{sent}`), false when `[mail]` is unset or delivery fails, delivery capped at 30s so a hung mail server never hangs the admin. #534 approval answers reuse it.
- The kernel returns `invite_link` beside the token (trusted base only, else null); the console emails it and reports `email_sent`, always showing the token as the fallback. Sync send: the admin already knows the address, so no oracle, and the flag answers truthfully.
