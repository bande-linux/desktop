FROM debian:13-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash dbus-daemon firefox-esr fonts-dejavu-core fonts-inter hicolor-icon-theme kmod libdrm2 libfontconfig1 libgbm1 libgl1-mesa-dri libinput10 \
        libseat1 libudev1 libvulkan1 libwayland-client0 libxkbcommon0 mesa-vulkan-drivers procps seatd udev \
    && rm -rf /var/lib/apt/lists/*
COPY init /sbin/unbound-guest-init
