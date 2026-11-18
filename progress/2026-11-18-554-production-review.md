# Production review fixes

Implemented bounded chat recall, provider retry recovery, managed enrollment, stream cleanup, and separation of pull request checks from release builds for #554. The hosted workflows run the Rust and desktop tests. The production build requires trusted runner registration and deployment bootstrap values. Release and authenticated Windows acceptance follow verification on the deployment.
