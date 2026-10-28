FROM debian:trixie
COPY *.deb /debs/
RUN apt-get update \
    && apt-get install -y --no-install-recommends dbus-bin dbus-daemon xdg-desktop-portal \
        /debs/unbound-config_*.deb /debs/xdg-desktop-portal-unbound_*.deb \
    && rm -rf /var/lib/apt/lists/*
COPY run.sh /usr/local/bin/run-portal-test
ENTRYPOINT ["/usr/local/bin/run-portal-test"]
