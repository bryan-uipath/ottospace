# A made-up seed shelf.
ROWS ?= 3
BEDS := $(wildcard beds/*.txt)

.PHONY: sow
sow: $(BEDS)
	@echo "sowing $(ROWS) rows"

ifeq ($(ROWS),0)
	$(error "no rows")
endif
