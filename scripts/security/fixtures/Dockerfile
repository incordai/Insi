# A preloaded immutable base is supplied by the lab builder.
ARG PYTHON_IMAGE
FROM ${PYTHON_IMAGE}
ARG LAB_MODE=vulnerable
ENV LAB_MODE=${LAB_MODE} PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
WORKDIR /lab
COPY app.py probe.py /lab/
USER 65534:65534
# Health is independent of the exploit probe and checks only the local service response.
HEALTHCHECK --interval=5s --timeout=2s --start-period=5s --retries=3 CMD python -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/?name=public', timeout=1)"
CMD ["python", "/lab/app.py"]
