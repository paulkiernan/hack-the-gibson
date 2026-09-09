# The Gibson - Makefile
#
# 'make'              build executable file 'gibson'
# 'make run'          build and launch the windowed app
# 'make scn'            build and launch the SceneKit preview window
# 'make saver'        build dist/Gibson.saver
# 'make install-saver'  install the screensaver for this user
# 'make clean'        remove objects, the executable, and the .saver bundle

CXX ?= c++
CXXFLAGS ?= -g -Wall -std=c++17 -Wno-deprecated-declarations
SAVER_MIN_OS ?= 11.0

# Prefer the local Homebrew prefix (Apple Silicon: /opt/homebrew, Intel: /usr/local)
# so we do not depend on a working `brew` API at build time.
ifneq ($(wildcard /opt/homebrew/opt/irrlicht),)
  BREW_PREFIX := /opt/homebrew
else ifneq ($(wildcard /usr/local/opt/irrlicht),)
  BREW_PREFIX := /usr/local
else
  BREW_PREFIX := $(shell brew --prefix 2>/dev/null)
endif

IRRLICHT_PREFIX ?= $(BREW_PREFIX)/opt/irrlicht

ifeq ($(wildcard $(IRRLICHT_PREFIX)/include/irrlicht/irrlicht.h),)
  $(error Irrlicht not found at $(IRRLICHT_PREFIX). Install with: brew install irrlicht)
endif

INCLUDE_DIRS = . include saver $(IRRLICHT_PREFIX)/include/irrlicht
INCLUDES = $(foreach d,$(INCLUDE_DIRS),-I$d)

LIB_DIRS = $(IRRLICHT_PREFIX)/lib $(BREW_PREFIX)/lib
LIB_NAMES = Irrlicht png jpeg z bz2 objc
FRAMEWORK_NAMES = OpenGL Cocoa IOKit Carbon Foundation AppKit CoreFoundation CoreGraphics

LIBS = \
	$(foreach d,$(LIB_DIRS),-L$d) \
	$(foreach n,$(LIB_NAMES),-l$n) \
	$(foreach n,$(FRAMEWORK_NAMES),-framework $n) \
	$(foreach d,$(LIB_DIRS),-Wl,-rpath,$(d))

PNG_A := $(wildcard $(BREW_PREFIX)/opt/libpng/lib/libpng.a)
JPEG_A := $(wildcard $(BREW_PREFIX)/opt/jpeg-turbo/lib/libjpeg.a)
ifeq ($(JPEG_A),)
  JPEG_A := $(wildcard $(BREW_PREFIX)/opt/jpeg/lib/libjpeg.a)
endif
IRRLICHT_A := $(IRRLICHT_PREFIX)/lib/libIrrlicht.a

SAVER_FRAMEWORKS = OpenGL Cocoa IOKit Carbon Foundation AppKit CoreFoundation CoreGraphics QuartzCore ScreenSaver
SAVER_LIBS = \
	-Wl,-ObjC \
	$(IRRLICHT_A) $(PNG_A) $(JPEG_A) \
	-lz -lbz2 -lobjc \
	$(foreach n,$(SAVER_FRAMEWORKS),-framework $n)

SCENE_SRCS = \
	PathCreator.cpp \
	cam.cpp \
	gibson_scene.cpp \
	pulse.cpp \
	room.cpp \
	gibson_settings.cpp \
	include/Effects/CEffectPostProc.cpp \
	include/Effects/CLensFlarePostProc.cpp \
	include/Effects/CRendererPostProc.cpp \
	include/Effects/CSplitPostProc.cpp \
	include/Effects/CTransitionPostProc.cpp \
	include/Effects/CWaterPostProc.cpp \
	include/Effects/IPostProc.cpp \
	include/HardwareSkinCallback.cpp \
	include/camera.cpp \
	include/events.cpp \
	include/globals.cpp \
	include/gui.cpp \
	include/lighting.cpp \
	include/mesh.cpp \
	include/ray3d.cpp \
	include/render.cpp

MM_SRCS = macos_retina.mm
SAVER_MM = saver/GibsonSaverView.mm
APP_SRCS = main.cpp
SCN_MM = \
	scenekit/GibsonSCNScene.mm \
	scenekit/GibsonSCNCamera.mm
PREVIEW_MM = scenekit/scn_preview.mm

SCENE_OBJS = $(SCENE_SRCS:.cpp=.o) $(MM_SRCS:.mm=.o)
APP_OBJS = $(APP_SRCS:.cpp=.o)
SCN_OBJS = $(SCN_MM:.mm=.o) gibson_settings.o
SAVER_OBJS = $(SAVER_MM:.mm=.o)
PREVIEW_OBJ = $(PREVIEW_MM:.mm=.o)
MAIN = gibson
PREVIEW = scn_preview

SCN_INCLUDES = -I. -Iscenekit -Isaver
SCN_FRAMEWORKS = SceneKit ModelIO QuartzCore Metal Cocoa Foundation AppKit CoreGraphics
SAVER_SCN_LIBS = \
	-Wl,-ObjC \
	$(foreach n,$(SCN_FRAMEWORKS) ScreenSaver,-framework $n)
PREVIEW_LIBS = \
	$(foreach n,$(SCN_FRAMEWORKS),-framework $n)

ROOT := $(abspath ..)
SAVER_DIR := $(ROOT)/dist/Gibson.saver
SAVER_BIN := $(SAVER_DIR)/Contents/MacOS/Gibson
INSTALL_DIR := $(HOME)/Library/Screen Savers

.PHONY: all clean run saver install-saver uninstall-saver enable-saver scn

all: $(MAIN)
	@echo "It's time to hack the gibson."

$(MAIN): $(SCENE_OBJS) $(APP_OBJS)
	$(CXX) $(CXXFLAGS) $(INCLUDES) -o $(MAIN) $(SCENE_OBJS) $(APP_OBJS) $(LIBS)

.cpp.o:
	$(CXX) $(CXXFLAGS) $(INCLUDES) -c $< -o $@

macos_retina.o: macos_retina.mm
	$(CXX) $(CXXFLAGS) $(INCLUDES) -c $< -o $@

scenekit/%.o: scenekit/%.mm
	$(CXX) $(CXXFLAGS) -fobjc-arc $(SCN_INCLUDES) -c $< -o $@

saver/GibsonSaverView.o: saver/GibsonSaverView.mm
	$(CXX) $(CXXFLAGS) -fobjc-arc $(SCN_INCLUDES) -c $< -o $@

run: $(MAIN)
	./$(MAIN)

$(PREVIEW): $(SCN_OBJS) $(PREVIEW_OBJ)
	$(CXX) $(CXXFLAGS) -fobjc-arc $(SCN_INCLUDES) -o $(PREVIEW) $(SCN_OBJS) $(PREVIEW_OBJ) $(PREVIEW_LIBS)

scn: $(PREVIEW)
	./$(PREVIEW)

saver: $(SAVER_BIN)

$(SAVER_BIN): $(SCN_OBJS) $(SAVER_OBJS) saver/Info.plist $(ROOT)/config.txt
	rm -rf "$(SAVER_DIR)"
	mkdir -p "$(SAVER_DIR)/Contents/MacOS" "$(SAVER_DIR)/Contents/Resources/media"
	cp saver/Info.plist "$(SAVER_DIR)/Contents/Info.plist"
	printf 'BNDL????' > "$(SAVER_DIR)/Contents/PkgInfo"
	rsync -a --exclude '*.xcf' --exclude 'screenshot*' "$(ROOT)/media/" "$(SAVER_DIR)/Contents/Resources/media/"
	cp "$(ROOT)/config.txt" "$(SAVER_DIR)/Contents/Resources/config.txt"
	$(CXX) $(CXXFLAGS) -fobjc-arc -mmacosx-version-min=$(SAVER_MIN_OS) -bundle \
		-o "$(SAVER_BIN)" $(SCN_OBJS) $(SAVER_OBJS) $(SAVER_SCN_LIBS)
	@if command -v codesign >/dev/null 2>&1; then \
		codesign --force --deep --sign - "$(SAVER_DIR)"; \
	fi
	@echo "Built $(SAVER_DIR)"
	@echo "Install with: make install-saver"

install-saver: saver
	mkdir -p "$(INSTALL_DIR)"
	rm -rf "$(INSTALL_DIR)/Gibson.saver"
	cp -R "$(SAVER_DIR)" "$(INSTALL_DIR)/Gibson.saver"
	mkdir -p "$(HOME)/Library/Application Support/TheGibson"
	@if [ ! -f "$(HOME)/Library/Application Support/TheGibson/config.txt" ]; then \
		cp "$(ROOT)/config.txt" "$(HOME)/Library/Application Support/TheGibson/config.txt"; \
		echo "Wrote $(HOME)/Library/Application Support/TheGibson/config.txt"; \
	fi
	@echo "Installed to $(INSTALL_DIR)/Gibson.saver"
	@echo "Open System Settings → Screen Saver and choose The Gibson."

enable-saver: install-saver
	python3 saver/enable_idle.py

uninstall-saver:
	rm -rf "$(INSTALL_DIR)/Gibson.saver"
	@echo "Removed $(INSTALL_DIR)/Gibson.saver"

clean:
	rm -f $(SCENE_OBJS) $(APP_OBJS) $(SAVER_OBJS) $(SCN_OBJS) $(PREVIEW_OBJ) $(MAIN) $(PREVIEW)
	rm -rf "$(SAVER_DIR)"
