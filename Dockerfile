# Disobey 2026 Badge Example Repo in a Docker Container
# Author: TenDRILLL
# Uses the specific repository from: tanelikaivola
# Check out how to run the examples from: https://github.com/tanelikaivola/disobey2026badge
# 
# Usage:
#    docker build . -t disobeybadge
#    docker run --device=/dev/ttyUSB0 -it disobeybadge

FROM rust:trixie
WORKDIR /app

RUN apt update && apt upgrade -y
RUN apt install -y ca-certificates curl git wget
RUN cargo install espup
RUN cargo install espflash

# For some reason, when doing espup install, it fails on xtensa components/cargos/whatev
# I'm no rust wizard, so I bypass this issue by downloading and installing the required things manually.
RUN wget -O /tmp/rust-src-1.92.0.0.tar.xz https://github.com/esp-rs/rust-build/releases/download/v1.92.0.0/rust-src-1.92.0.0.tar.xz
RUN wget -O /tmp/rust-1.92.0.0-x86_64-unknown-linux-gnu.tar.xz https://github.com/esp-rs/rust-build/releases/download/v1.92.0.0/rust-1.92.0.0-x86_64-unknown-linux-gnu.tar.xz

RUN git clone https://github.com/tanelikaivola/disobey2026badge .

RUN mkdir -p /opt/xtensa && \
tar -xf /tmp/rust-1.92.0.0-x86_64-unknown-linux-gnu.tar.xz -C /opt/xtensa && \
tar -xf /tmp/rust-src-1.92.0.0.tar.xz -C /opt/xtensa

RUN /opt/xtensa/rust-nightly-x86_64-unknown-linux-gnu/install.sh --prefix=/usr/local --without=rust-docs
RUN /opt/xtensa/rust-src-nightly/install.sh --prefix=/usr/local --without=rust-docs

RUN rustup toolchain link esp /usr/local

RUN espup install -l debug

ENTRYPOINT ["/bin/bash", "-c", "source /root/export-esp.sh; exec bash"]
