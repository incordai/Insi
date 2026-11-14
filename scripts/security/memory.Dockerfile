# Native-memory controls compile and run only in a disposable Linux image.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends gcc libc6-dev
COPY memory_probe.c /src/probe.c
RUN gcc -g -O0 -fno-omit-frame-pointer -fsanitize=address,undefined /src/probe.c -o /probe-leak \
    && gcc -g -O0 -fno-omit-frame-pointer -fsanitize=address,undefined -DFIXED /src/probe.c -o /probe-fixed
USER 65534:65534
ENV ASAN_OPTIONS=detect_leaks=1:exitcode=23 LSAN_OPTIONS=exitcode=23
CMD ["/probe-fixed"]
