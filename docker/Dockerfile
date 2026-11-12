# Foreman headless server. Built by .github/workflows/release.yml from the same
# release archives as the GitHub Release (no compile inside Docker), for
# linux/amd64 and linux/arm64:
#   staging/<amd64|arm64>/  = an unpacked foreman-server-<target>.tar.xz
#   staging/data/           = an empty folder (becomes /data, owned by nonroot)
#
#   docker run -d -p 9090:9090 -p 7912:7912 -v foreman-data:/data ghcr.io/hellsparks/foreman
#
# distroless/cc: glibc + libssl3 + CA certificates, no shell, runs as uid 65532.
FROM gcr.io/distroless/cc-debian12:nonroot
ARG TARGETARCH
COPY --chown=nonroot:nonroot staging/${TARGETARCH}/ /app/
COPY --chown=nonroot:nonroot staging/data/ /data/
ENV FOREMAN__HOST=0.0.0.0 \
    FOREMAN__DATA_DIR=/data \
    FOREMAN_CONFIG_FILE=/data/config.toml
VOLUME ["/data"]
EXPOSE 9090 7912
ENTRYPOINT ["/app/foreman-server"]
