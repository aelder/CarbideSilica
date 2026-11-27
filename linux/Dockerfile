# Build and test image for the Linux app. The app is built and run only in this container.
# package.sh also makes the packages here: dpkg-dev (dpkg-deb, dpkg-shlibdeps), fakeroot,
# rsvg-convert (icon sizes), desktop-file-validate and appstreamcli.
# Debian 13 (trixie) has GTK 4.18. Rust comes from rustup.
FROM debian:trixie

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl build-essential pkg-config \
        libgtk-4-dev libasound2-dev libfontconfig-dev \
        xvfb xauth imagemagick x11-utils xdotool xclip \
        fontconfig dbus-x11 \
        fonts-noto-color-emoji fonts-noto-cjk fonts-noto-core \
        dpkg-dev fakeroot librsvg2-bin desktop-file-utils appstream \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.91.1 \
    && rustc --version

# No accessibility bus in the container: without this, GTK waits for it at start.
ENV NO_AT_BRIDGE=1 GTK_A11Y=none
WORKDIR /src/linux
