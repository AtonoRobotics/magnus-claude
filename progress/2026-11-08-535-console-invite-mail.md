# #535 console invites email again (2026-11-08)

- `api.rs`: `GET /invite` + `POST /invite` (form, mismatch/short re-render, spent shell); `link_base` is `pub(crate)`; cookie guard exempts POST /invite.
- `accounts.rs`: `invite_live` peek.
- `control.rs`: `POST /v1/admin/mail` (console-only send, 30s cap); `admin_invite`/`admin_resend_invite` return `invite_link` (null without a trusted base).
- `admin/users.rs`: invite/resend email the link via `/v1/admin/mail`, report `email_sent`, always show the token; resend gains `email_sent`.
- `desktop/admin/admin.js`: the token dialog says emailed when it was, and the invite form no longer claims no mail.
- Tests: `an_invitation_token_reads_live_until_it_is_used` unit (green locally with the accept tests); `an_invite_is_emailed_and_accepted_behind_its_link` e2e (emailed link round trip, resend rotation, page accept) runs in CI per user preference.
- Next: #534 (self sign-up with admin approval).
