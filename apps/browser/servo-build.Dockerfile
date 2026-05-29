# Host-agnostic Servo build recipe (Railway / Modal / RunPod / local Docker).
#
# Validates the Servo build environment OFF the developer machine. Headless;
# no window. Pin a known-good Servo revision before relying on this.
#
# Host notes (Modal is NOT a host: deprecated from the stack as of 2026-05-29):
# - GitHub Actions (current): the working CI build host once Actions billing was
#   fixed; see .github/workflows/servo-browser.yml. Preferred for the headless
#   build/verify.
# - Railway (fallback): a full Servo build at IMAGE-BUILD time (the RUN mach build
#   below) may exceed Railway's build-time / image-size limits. If using Railway,
#   move `mach build` to the container CMD (runtime) so it runs on service
#   resources rather than the build step, and size the service plan accordingly.
# - Local Docker: build at image time is fine; runs on the dev machine.
#
# Disk: a Servo --dev target dir exceeds ~14GB. The GH Actions run failed at
# "No space left on device" until ~20-25GB of preinstalled SDKs were reclaimed.
# Any host (Railway/local) needs >~25GB free for the build dir; the workflow
# also sets CARGO_PROFILE_DEV_DEBUG=0 to drop debuginfo (the bulk of the size).

FROM ubuntu:24.04

# Validation build drops debuginfo to keep the target dir small (see disk note).
ENV CARGO_PROFILE_DEV_DEBUG=0

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

# mach loads command modules (bootstrap_commands.py imports toml) with the
# invoking Python before activating its managed venv, so install mach's Python
# deps into the runner Python first (fixes "No module named 'toml'").
RUN python3 -m pip install --break-system-packages toml \
    && if [ -f python/requirements.txt ]; then python3 -m pip install --break-system-packages -r python/requirements.txt; fi

# Heavy step. In GitHub Actions / local Docker, keep it here (build time). On
# Railway, move this to the CMD so it runs at container runtime, not the build step.
RUN python3 ./mach build --dev

# v2: also build the theorem-browser embedder (apps/browser) as an external
# consumer of the servo crate, and run the headless substrate-seam test.
CMD ["bash", "-lc", "echo 'servo build image ready'"]
