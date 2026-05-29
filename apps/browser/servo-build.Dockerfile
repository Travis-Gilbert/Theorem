# Host-agnostic Servo build recipe (Railway / Modal / RunPod / local Docker).
#
# Validates the Servo build environment OFF the developer machine. Headless;
# no window. Pin a known-good Servo revision before relying on this.
#
# Host notes:
# - Modal (recommended for the heavy build): the `mach build` belongs in the
#   image build; Modal is built for heavy compute + scale-to-zero. Adapt this
#   into a Modal image (it mirrors these apt + rustup + mach steps).
# - Railway: a full Servo build at IMAGE-BUILD time (the RUN mach build below)
#   may exceed Railway's build-time / image-size limits. If using Railway, move
#   `mach build` to the container CMD (runtime) so it runs on service resources
#   rather than the build step, and size the service plan accordingly.
# - GitHub Actions: blocked on this account by Actions billing as of 2026-05-29.

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        git curl python3 python3-pip sudo build-essential pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Rust via rustup; Servo pins its own toolchain via rust-toolchain.toml.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /build
# Pin a specific rev (replace main) before relying on this build for reproducibility.
RUN git clone --depth 1 https://github.com/servo/servo.git

WORKDIR /build/servo
# mach bootstrap installs the remaining platform build dependencies.
RUN python3 ./mach bootstrap --force || python3 ./mach bootstrap

# Heavy step. On Modal/local-Docker keep it here (image build). On Railway move
# this to the CMD so it runs at container runtime instead of the build step.
RUN python3 ./mach build --dev

# v2: also build the theorem-browser embedder (apps/browser) as an external
# consumer of the servo crate, and run the headless substrate-seam test.
CMD ["bash", "-lc", "echo 'servo build image ready'"]
