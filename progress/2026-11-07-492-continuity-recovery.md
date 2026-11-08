# Continuity recovery and deliberate scrolling (#482, #492)

The full #463 feature is carried from the failed #479 onto current master. Two review findings are corrected: late rejection updates a remounted composer before navigation can overwrite recovered text, and recovered thread drafts retain their entry when acknowledgement leaves newer edits/attachments.

Real-appliance Chromium continuity suite: 2/2 passed. Added navigation/remount rejection, accepted secondary draft with newer text and an actual uploaded attachment, restart and named-entry reopening. A controlled observer-delivery test failed with the old animation-frame scroll suppression and passes when only the restored offset is ignored. Existing distinct drafts, uploads, IME, uncertain send/restart, work focus/history, anchor, unavailable storage and sign-out tests remain intact.

Real installed Linux/WebKit: completed task links passed after accepting review under Needs you and after adding later conversation content. The test uses actual stored task/message state and real native clicks. It is an additional focused check; the original KVM-dependent release flow that failed in CI 42438300477 still must pass in full CI. Hit-test diagnostics preserve evidence if a native click remains blocked; no forced script click or retry masks the behavior.
