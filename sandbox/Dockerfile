# The sandbox guest root filesystem. Built by build.sh; mounted read-only by
# every microVM, with a per-run writable /workspace disk.
FROM python:3.12-slim-bookworm AS rootfs
RUN apt-get update && apt-get install -y --no-install-recommends \
      coreutils findutils grep sed gawk procps file less zip unzip xz-utils bzip2 \
      poppler-utils qpdf ghostscript imagemagick tesseract-ocr \
      libreoffice-writer libreoffice-calc libreoffice-impress fonts-dejavu fonts-liberation fonts-noto-core \
      xvfb xdotool x11-utils x11-xserver-utils fluxbox xterm \
      sqlite3 git jq \
    && rm -rf /var/lib/apt/lists/*
RUN pip install --no-cache-dir \
      python-docx openpyxl xlsxwriter python-pptx reportlab pypdf pdfplumber pymupdf \
      pandas numpy matplotlib pillow markdown beautifulsoup4 lxml tabulate jinja2 \
      scipy scikit-learn pytest
RUN mkdir -p /workspace /sbin
COPY magnus-guest /sbin/magnus-guest
COPY sitecustomize.py /usr/local/lib/python3.12/site-packages/sitecustomize.py
COPY check_tools.py /usr/local/lib/magnus/check_tools.py

# The browser microVM: Chromium and the sidecar, no network device. Its only
# way out is the guest's proxy forwarder to the kernel's egress proxy.
FROM debian:bookworm-slim AS browser-rootfs
RUN apt-get update && apt-get install -y --no-install-recommends \
      chromium nodejs npm ca-certificates fonts-dejavu fonts-liberation fonts-noto-core \
    && rm -rf /var/lib/apt/lists/*
COPY browser/package.json browser/package-lock.json browser/sidecar.js /opt/magnus/
RUN cd /opt/magnus && npm ci --omit=dev && rm -rf /root/.npm
RUN mkdir -p /sbin /run /tmp
COPY magnus-guest /sbin/magnus-guest

FROM debian:bookworm-slim AS pack
RUN apt-get update && apt-get install -y --no-install-recommends e2fsprogs && rm -rf /var/lib/apt/lists/*
COPY --from=rootfs / /rootfs
RUN rm -rf /rootfs/proc/* /rootfs/sys/* /rootfs/dev/* && : > /rootfs/etc/resolv.conf && \
    mke2fs -q -t ext4 -L rootfs -d /rootfs -F /rootfs.ext4 4G
COPY --from=browser-rootfs / /browser
RUN rm -rf /browser/proc/* /browser/sys/* /browser/dev/* && : > /browser/etc/resolv.conf && \
    mke2fs -q -t ext4 -L browser -d /browser -F /browser-rootfs.ext4 3G

FROM scratch AS export
COPY --from=pack /rootfs.ext4 /rootfs.ext4
COPY --from=pack /browser-rootfs.ext4 /browser-rootfs.ext4
