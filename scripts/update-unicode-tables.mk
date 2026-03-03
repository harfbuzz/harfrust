#!/usr/bin/env -S make -f

HARFBUZZ_DIR ?= $(HOME)/harfbuzz
HB_SRC := $(HARFBUZZ_DIR)/src
PACKTAB_DIR ?= $(HOME)/packtab
PACKTAB_PYTHONPATH ?= $(PACKTAB_DIR)
PYTHON ?= python3

HARFRUST_HB_DIR := ../harfrust/src/hb

GENERATED := \
	$(HARFRUST_HB_DIR)/ucd_table.rs \
	$(HARFRUST_HB_DIR)/ot_shaper_use_table.rs \
	$(HARFRUST_HB_DIR)/ot_shaper_arabic_table.rs \
	$(HARFRUST_HB_DIR)/ot_shaper_indic_table.rs \
	$(HARFRUST_HB_DIR)/unicode_emoji_table.rs \
	$(HARFRUST_HB_DIR)/tag_table.rs \
	$(HARFRUST_HB_DIR)/ot_shaper_vowel_constraints.rs

.PHONY: all rust hb-refresh clean

all: rust

rust: $(GENERATED)

# HarfBuzz's update-unicode-tables.make is path-sensitive and owns its own
# targets, so use it via a recursive make instead of including it directly.
hb-refresh:
	$(MAKE) -C $(HB_SRC) -f update-unicode-tables.make all

$(HARFRUST_HB_DIR)/ucd_table.rs: $(HB_SRC)/gen-ucd-table.py $(HB_SRC)/ucd.nounihan.grouped.zip $(HB_SRC)/hb-script-list.h
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(word 2,$^) $(word 3,$^) > $@ || ($(RM) $@; false)

$(HARFRUST_HB_DIR)/ot_shaper_use_table.rs: $(HB_SRC)/gen-use-table.py $(HB_SRC)/IndicSyllabicCategory.txt $(HB_SRC)/IndicPositionalCategory.txt $(HB_SRC)/ArabicShaping.txt $(HB_SRC)/DerivedCoreProperties.txt $(HB_SRC)/UnicodeData.txt $(HB_SRC)/Blocks.txt $(HB_SRC)/Scripts.txt $(HB_SRC)/ms-use/IndicSyllabicCategory-Additional.txt $(HB_SRC)/ms-use/IndicPositionalCategory-Additional.txt
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,10,$^) > $@ || ($(RM) $@; false)

$(HARFRUST_HB_DIR)/ot_shaper_arabic_table.rs: $(HB_SRC)/gen-arabic-table.py $(HB_SRC)/ArabicShaping.txt $(HB_SRC)/UnicodeData.txt $(HB_SRC)/Blocks.txt
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,4,$^) > $@ || ($(RM) $@; false)

$(HARFRUST_HB_DIR)/ot_shaper_indic_table.rs: $(HB_SRC)/gen-indic-table.py $(HB_SRC)/IndicSyllabicCategory.txt $(HB_SRC)/IndicPositionalCategory.txt $(HB_SRC)/Blocks.txt
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,4,$^) > $@ || ($(RM) $@; false)

$(HARFRUST_HB_DIR)/unicode_emoji_table.rs: $(HB_SRC)/gen-emoji-table.py $(HB_SRC)/emoji-data.txt $(HB_SRC)/emoji-test.txt
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,3,$^) > $@ || ($(RM) $@; false)

$(HARFRUST_HB_DIR)/tag_table.rs: $(HB_SRC)/gen-tag-table.py $(HB_SRC)/languagetags $(HB_SRC)/language-subtag-registry
	PYTHONDONTWRITEBYTECODE=1 \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,3,$^) > $@ || ($(RM) $@; false)

$(HARFRUST_HB_DIR)/ot_shaper_vowel_constraints.rs: $(HB_SRC)/gen-vowel-constraints.py $(HB_SRC)/ms-use/IndicShapingInvalidCluster.txt $(HB_SRC)/Scripts.txt
	PYTHONDONTWRITEBYTECODE=1 \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,3,$^) > $@ || ($(RM) $@; false)
	rustfmt $@

clean:
	$(RM) $(GENERATED)
