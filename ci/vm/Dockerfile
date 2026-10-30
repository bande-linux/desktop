FROM debian:13-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash dbus-daemon firefox-esr fonts-dejavu-core fonts-inter fonts-jetbrains-mono fonts-manrope hicolor-icon-theme kmod libdrm2 libfontconfig1 libgbm1 libgl1-mesa-dri libinput10 libpam-modules libpam-runtime \
        libseat1 libudev1 libvulkan1 libwayland-client0 libxkbcommon0 mesa-vulkan-drivers pipewire procps seatd udev wireplumber \
    && rm -rf /var/lib/apt/lists/*
RUN echo 'root:unbound-test' | chpasswd
COPY init /sbin/unbound-guest-init
