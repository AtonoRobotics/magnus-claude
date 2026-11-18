# Production review fixes

- Ordinary chat keeps recent entry identifiers and assembles up to 24 entries and 8 KiB of prior text from visible history. Short excerpts from other conversations have a separate 1 KiB limit. The memory pass reads the exchange that just ended plus up to 2 KiB of known facts. A chat task still carries the source it needs in its contract.
- The task log records a bounded retry deadline after transient provider failures. The appliance wakes the run when due and applies the same model-call and cost limits. Permanent refusals retain their specific reason.
- The HTTP response owns the live subscription. The kernel closes it when the run stops permanently. A reconnect can replace the renderer's old partial text from a snapshot.
- A managed installer pins the deployment's private DNS origin using values from the signed build. Existing sign-in and device pairing follow a successful TLS connection.
- Pull requests run on hosted runners. The release workflow builds its own artifacts from trusted refs. GitHub runner registration must enforce the same boundary outside the workflow.
