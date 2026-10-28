FROM debian:trixie
RUN apt-get update \
    && apt-get install -y --no-install-recommends dbus-bin dbus-daemon dbus-system-bus-common polkitd passwd \
    && rm -rf /var/lib/apt/lists/*
COPY org.unbound.polkit-test.policy /usr/share/polkit-1/actions/
COPY run.sh /usr/local/bin/run-polkitd-test
ENTRYPOINT ["/usr/local/bin/run-polkitd-test"]
