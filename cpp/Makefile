# Sidera 构建（宿主机 amd64；aarch64 见 README / 手写交叉命令）
# WITH_WAYLAND=auto/0/1；需要 wayland-client、xkbcommon 与 wayland-scanner。
CXX      ?= g++
CC       ?= gcc
CXXFLAGS ?= -std=c++17 -O2 -fPIC
PKGS      = Qt5Core Qt5Gui Qt5Widgets Qt5Network
QTFLAGS   = $(shell pkg-config --cflags $(PKGS))
QTLIBS    = $(shell pkg-config --libs $(PKGS))
LIBS      = -lX11 -lxcb -lXext -lXtst
DEFS      =
INC       =

CPP_SRC = src/main.cpp src/widget.cpp src/sidebar.cpp src/drawing.cpp \
          src/modes.cpp src/wps.cpp src/popups.cpp src/splash.cpp src/settings.cpp \
          src/wps_bridge.cpp
HDR = $(wildcard src/*.h)

# ---- Wayland 后端（可选） ----
WL_SCANNER := $(shell command -v wayland-scanner 2>/dev/null)
WITH_WAYLAND ?= auto
WL_AVAILABLE := $(shell pkg-config --exists wayland-client xkbcommon && test -n "$(WL_SCANNER)" && echo 1)
ifeq ($(WITH_WAYLAND),1)
  ifneq ($(WL_AVAILABLE),1)
    $(error WITH_WAYLAND=1 requires wayland-client, xkbcommon and wayland-scanner)
  endif
endif
ifeq ($(filter $(WITH_WAYLAND),auto 0 1),)
  $(error WITH_WAYLAND must be auto, 0 or 1)
endif
HAVE_WL := $(if $(filter 0,$(WITH_WAYLAND)),,$(WL_AVAILABLE))
WL_C       =
WL_H       =
ifeq ($(HAVE_WL),1)
  LIBS    += -lwayland-client -lxkbcommon
  DEFS    += -DSIDERA_HAVE_WAYLAND
  INC     += -Ibuild
  CPP_SRC += src/backend_wayland.cpp
  WL_C     = build/wlr-layer-shell-unstable-v1-protocol.c build/xdg-shell-protocol.c \
             build/fractional-scale-v1-protocol.c build/viewporter-protocol.c \
             build/virtual-keyboard-unstable-v1-protocol.c build/wlr-screencopy-unstable-v1-protocol.c
  WL_H     = build/wlr-layer-shell-unstable-v1-client-protocol.h build/xdg-shell-client-protocol.h \
             build/fractional-scale-v1-client-protocol.h build/viewporter-client-protocol.h \
             build/virtual-keyboard-unstable-v1-client-protocol.h build/wlr-screencopy-unstable-v1-client-protocol.h
endif

C_OBJS   = $(WL_C:.c=.o)
CPP_OBJS = $(CPP_SRC:.cpp=.o) build/startup_art.o

# 输出文件名（在 aarch64 容器里用 make OUT=annotate_aarch64）
OUT ?= annotate_amd64

all: $(OUT)

build/startup_art.cpp: src/startup.qrc ../assets/source/startup-ink.png
	@mkdir -p build
	rcc -no-compress -name sidera_startup -o $@ $<

build/wlr-layer-shell-unstable-v1-client-protocol.h: ../protocols/wlr-layer-shell-unstable-v1.xml
	@mkdir -p build
	$(WL_SCANNER) client-header $< $@

build/wlr-layer-shell-unstable-v1-protocol.c: ../protocols/wlr-layer-shell-unstable-v1.xml
	@mkdir -p build
	$(WL_SCANNER) private-code $< $@

build/xdg-shell-client-protocol.h: ../protocols/xdg-shell.xml
	@mkdir -p build
	$(WL_SCANNER) client-header $< $@

build/xdg-shell-protocol.c: ../protocols/xdg-shell.xml
	@mkdir -p build
	$(WL_SCANNER) private-code $< $@

build/fractional-scale-v1-client-protocol.h: ../protocols/fractional-scale-v1.xml
	@mkdir -p build
	$(WL_SCANNER) client-header $< $@

build/fractional-scale-v1-protocol.c: ../protocols/fractional-scale-v1.xml
	@mkdir -p build
	$(WL_SCANNER) private-code $< $@

build/viewporter-client-protocol.h: ../protocols/viewporter.xml
	@mkdir -p build
	$(WL_SCANNER) client-header $< $@

build/viewporter-protocol.c: ../protocols/viewporter.xml
	@mkdir -p build
	$(WL_SCANNER) private-code $< $@

build/virtual-keyboard-unstable-v1-client-protocol.h: ../protocols/virtual-keyboard-unstable-v1.xml
	@mkdir -p build
	$(WL_SCANNER) client-header $< $@

build/virtual-keyboard-unstable-v1-protocol.c: ../protocols/virtual-keyboard-unstable-v1.xml
	@mkdir -p build
	$(WL_SCANNER) private-code $< $@

build/wlr-screencopy-unstable-v1-client-protocol.h: ../protocols/wlr-screencopy-unstable-v1.xml
	@mkdir -p build
	$(WL_SCANNER) client-header $< $@

build/wlr-screencopy-unstable-v1-protocol.c: ../protocols/wlr-screencopy-unstable-v1.xml
	@mkdir -p build
	$(WL_SCANNER) private-code $< $@

# A feature/toolchain change must invalidate old objects, including main.o.
build/config: FORCE
	@mkdir -p build
	@printf '%s\n' '$(CXX) $(CXXFLAGS) $(LDFLAGS) $(DEFS) $(QTFLAGS) $(INC) $(LIBS)' > $@.tmp
	@cmp -s $@.tmp $@ && rm $@.tmp || mv $@.tmp $@

%.o: %.cpp $(HDR) $(WL_H) build/config
	$(CXX) $(CXXFLAGS) $(DEFS) $(QTFLAGS) $(INC) -c $< -o $@

%.o: %.c
	$(CC) -O2 -fPIC -c $< -o $@

$(OUT): $(CPP_OBJS) $(C_OBJS)
	$(CXX) $(LDFLAGS) $(CPP_OBJS) $(C_OBJS) $(QTLIBS) $(LIBS) -o $@

clean:
	rm -f annotate_amd64 annotate_aarch64
	rm -f src/*.o build/*.o
	rm -rf build

test:
	python3 tests/test_bridge_api.py

test-lifecycle:
	python3 tests/test_bridge_lifecycle.py

.PHONY: all clean test test-lifecycle FORCE
