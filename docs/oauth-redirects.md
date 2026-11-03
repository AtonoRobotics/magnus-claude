# Redirects to register for each provider

Magnus sends people to the provider's own sign-in page and catches the
answer on the computer where the desktop app is running (RFC 8252). The
address is chosen when they press Connect:

`http://127.0.0.1:<port>/callback`

`<port>` is a free port on that computer. It is not 8080, and it is not
`localhost`. A console entry of `http://localhost:8080/oauth/<provider>/callback`
does not match, so the provider refuses the redirect.

Register the loopback form below. Where a provider accepts any port on
`127.0.0.1`, one saved callback is enough. Where it requires the port as
well, this build cannot name the port in advance; that connector stays
blocked until the provider allows loopback ports or the relay callback
exists (`https://pyrallon.ai/v1/connections/callback`, the product spec's
web client, which this appliance does not serve yet).

After this change, each person signs in to Google once more. A
testing-mode refresh token lasts 7 days; when it dies, the card says the
sign-in has expired and asks them to sign in again.

| Provider | Register this callback | Scopes the sign-in asks for | Notes |
| --- | --- | --- | --- |
| Google | Desktop client: any port on `http://127.0.0.1/callback` is accepted. Do not switch this client to a web client until the relay exists. | openid, email, profile, plus the product: `https://mail.google.com/`, gmail.modify, drive, calendar, documents, spreadsheets. Chat also asks for the four `chat.*` scopes, which are not on the consent screen yet. | Calls Gmail v1, Drive v3, Calendar v3, Docs v1, Sheets v4, Chat v1, and People v1 to name the account. Not the Workspace MCP preview hosts. |
| Microsoft | `http://127.0.0.1/callback` if the app registration allows a public loopback; otherwise the port must match and this is blocked. | openid, profile, email, offline_access, User.Read, Mail.Read, Calendars.Read, Files.Read | Graph `https://graph.microsoft.com/v1.0`. Directory is `organizations` unless `MICROSOFT_TENANT_ID` is set. |
| GitHub | `http://127.0.0.1/callback` (no port). GitHub then accepts any port with that path. | `read:user`, `repo` | `https://api.github.com`. The token can change repositories; the tools only read. |
| Slack | Exact URL, and Slack does not ignore the port. Blocked for a finished sign-in until the app allows `http://127.0.0.1:<any>/callback` or the relay exists. | user scopes `users:read`, `team:read`, `channels:read`, `groups:read`, `im:read` (sent as `user_scope`) | `https://slack.com/api`. |
| Vercel | Exact callback. Same loopback limit as Slack. | openid, email, profile, offline_access | `https://vercel.com/oauth/authorize`, token `https://api.vercel.com/login/oauth/token`. |
| Sentry | Exact callback. Same loopback limit. | org:read, project:read, event:read | `https://sentry.io/api/0`. |
| Asana | Exact callback. Same loopback limit. | none in the request; Asana uses the permission set on the app | `https://app.asana.com/api/1.0`. |
| GitLab | Exact callback on gitlab.com. Same loopback limit. | read_user, read_api | `https://gitlab.com/api/v4`. |
| Hugging Face | Exact callback. Same loopback limit. | openid, profile, email | `https://huggingface.co/api`. |
| Supabase | Exact callback. The management API's scope parameter is deprecated; set access on the OAuth app. | none in the request | `https://api.supabase.com/v1`. Beta authorize endpoint. |
| Cloudflare | Exact redirect on the OAuth client, and the client must allow `user.read`, `account.read`, `zone.read`. | those three | `https://api.cloudflare.com/client/v4`. `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` are stored in the vault and are not used as a person's sign-in. |
| Shopify | Not connected. The authorize URL is `https://{shop}.myshopify.com/admin/oauth/authorize`, and the shop is not known until someone names it. | would be read scopes on the app (`read_products` and what the shop should allow) | Secret can be stored (`SHOPIFY_CLIENT_ID`, `SHOPIFY_CLIENT_SECRET`). No tool is offered. |

Not registered, so not in the catalog. Each would need a confidential client, the same loopback callback, and a secret in the repository before a connector is added:

| Provider | What it would need |
| --- | --- |
| Figma | OAuth at `https://www.figma.com/oauth`, scopes `file_content:read` (and `file_comments:read` to read comments). Redirect as above. |
| Stripe | A restricted key is not a person's sign-in. Connect OAuth is `https://connect.stripe.com/oauth/authorize` with `read_write` or narrower. The product spec's Link wallet is a separate application. |
| Notion | Public integration, redirect as above, capabilities the integration page ticks (read content). API `https://api.notion.com/v1`. |
| Miro | OAuth at `https://miro.com/oauth/authorize`, scopes `boards:read`. |
| Canva | Canva Connect, `https://www.canva.com/api/oauth/authorize`, scopes `design:meta:read` and `design:content:read`. |

The deployment reads `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `MICROSOFT_CLIENT_ID`, `MICROSOFT_CLIENT_SECRET`, `MICROSOFT_TENANT_ID`, `GH_OAUTH_CLIENT_ID`, `GH_OAUTH_CLIENT_SECRET`, and `VERCEL_`, `SLACK_`, `SENTRY_`, `ASANA_`, `CLOUDFLARE_`, `GITLAB_`, `HUGGINGFACE_`, `SUPABASE_` and `SHOPIFY_` `CLIENT_ID` / `CLIENT_SECRET`, plus `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`, into the vault on each master build. A name that is unset is skipped.
