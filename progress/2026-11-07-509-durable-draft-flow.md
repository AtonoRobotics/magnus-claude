# Stable new-conversation flow

Claimed #509 for the red-master browser-flow race after v1.2 integration #483 merged. Updated only `desktop/tests/flow.mjs` to wait for the persistent draft, then assert its named navigation entry and empty conversation/composer. The timeout remains 60 seconds; every existing end-to-end outcome assertion remains.

`node --check desktop/tests/flow.mjs` passes. Real-appliance draft/attachment/send-recovery browser checks pass (2/2, 12.47s) against merged master using isolated target artifacts. Both full sign-in flows ran locally and authenticated, but stopped before the changed step because the sandbox guest path was absent and this host has no usable KVM device. They are not reported as passing. Full CI must run the original real KVM/Keycloak end-to-end flows and installed-host matrix. The preceding master failure provides the unmodified-test regression evidence. No product assets changed, so the final Windows build proof from #483 remains applicable.
