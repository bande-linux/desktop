FROM debian:13-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        dbus-daemon fonts-dejavu-core kmod libdrm2 libgbm1 libinput10 libseat1 \
        libudev1 libvulkan1 libxkbcommon0 mesa-vulkan-drivers procps seatd udev \
    && rm -rf /var/lib/apt/lists/*
COPY init /sbin/unbound-guest-init
