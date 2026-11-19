# Toolchain image for Handover CI jobs that need GTK, GStreamer and a
# disposable desktop stack. Publishing it keeps hosted runners from
# spending minutes in apt-get against a degraded package mirror.
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --no-install-recommends -y \
        build-essential \
        ca-certificates \
        cmake \
        curl \
        dbus-daemon \
        dbus-user-session \
        ffmpeg \
        git \
        gnome-keyring \
        gstreamer1.0-libav \
        gstreamer1.0-plugins-base \
        gstreamer1.0-plugins-good \
        heif-gdk-pixbuf \
        iproute2 \
        libadwaita-1-dev \
        libgtk-4-dev \
        libgtk-4-media-gstreamer \
        libheif-plugin-aomdec \
        libheif-plugin-libde265 \
        libssl-dev \
        pkg-config \
        playerctl \
        sudo \
        webp-pixbuf-loader \
        xdotool \
        xvfb \
    && rm -rf /var/lib/apt/lists/*
