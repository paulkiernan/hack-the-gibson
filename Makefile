# Convenience wrapper so `make` works from the project root.
.PHONY: all clean run saver install-saver uninstall-saver enable-saver scn

all:
	$(MAKE) -C src

clean:
	$(MAKE) -C src clean

run:
	$(MAKE) -C src run

scn:
	$(MAKE) -C src scn

saver:
	$(MAKE) -C src saver

install-saver:
	$(MAKE) -C src install-saver

enable-saver:
	$(MAKE) -C src enable-saver

uninstall-saver:
	$(MAKE) -C src uninstall-saver
