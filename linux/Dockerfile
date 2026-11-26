# Build and test image for the Linux app. The app is built and run only in this container.
# Debian 13 (trixie) has GTK 4.18. Rust comes from rustup.
FROM debian:trixie

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl build-essential pkg-config \
        libgtk-4-dev libasound2-dev libfontconfig-dev \
        xvfb xauth imagemagick x11-utils xdotool \
        fontconfig dbus-x11 \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.91.1 \
    && rustc --version

# No accessibility bus in the container: without this, GTK waits for it at start.
ENV NO_AT_BRIDGE=1 GTK_A11Y=none
WORKDIR /src/linux
