FROM debian:13-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash dbus-daemon firefox-esr fonts-dejavu-core fonts-inter gstreamer1.0-pipewire gstreamer1.0-plugins-good gstreamer1.0-plugins-ugly gstreamer1.0-tools hicolor-icon-theme kmod libdrm2 libfontconfig1 libgbm1 libgl1-mesa-dri libinput10 libpam-modules libpam-runtime \
        libseat1 libudev1 libvulkan1 libwayland-client0 libxkbcommon0 mesa-vulkan-drivers pipewire procps seatd udev wireplumber xdg-desktop-portal \
    && rm -rf /var/lib/apt/lists/*
RUN echo 'root:unbound-test' | chpasswd
COPY init /sbin/unbound-guest-init
