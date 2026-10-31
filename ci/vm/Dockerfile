FROM debian:13-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash dbus-daemon dbus-user-session firefox-esr fonts-dejavu-core fonts-inter fonts-jetbrains-mono fonts-manrope greetd gstreamer1.0-pipewire gstreamer1.0-plugins-good gstreamer1.0-plugins-ugly gstreamer1.0-tools hicolor-icon-theme kmod libdrm2 libfontconfig1 libgbm1 libgl1-mesa-dri libinput10 libpam-modules libpam-runtime libpam-systemd \
        libseat1 libudev1 libvulkan1 libwayland-client0 libxkbcommon0 mesa-vulkan-drivers pipewire procps seatd systemd udev wireplumber xdg-desktop-portal \
    && rm -rf /var/lib/apt/lists/*
RUN echo 'root:bande-test' | chpasswd \
    && rm -f /etc/systemd/system/display-manager.service
COPY init /sbin/bande-guest-init
COPY login.service /etc/systemd/system/bande-guest-login.service
