#!/usr/bin/env -S make -f

HARFBUZZ_DIR ?= $(HOME)/harfbuzz
HB_SRC := $(HARFBUZZ_DIR)/src
PACKTAB_DIR ?= $(HOME)/packtab
PACKTAB_PYTHONPATH ?= $(PACKTAB_DIR)
PYTHON ?= python3

HARFRUST_SRC_DIR := ../harfrust/src
HARFRUST_UNICODE_DIR := ../harfrust/src/unicode

GENERATED := \
	$(HARFRUST_UNICODE_DIR)/ucd_table.rs \
	$(HARFRUST_SRC_DIR)/ot/shaper/use_table.rs \
	$(HARFRUST_SRC_DIR)/ot/shaper/arabic_table.rs \
	$(HARFRUST_SRC_DIR)/ot/shaper/arabic_pua.rs \
	$(HARFRUST_SRC_DIR)/ot/shaper/indic_table.rs \
	$(HARFRUST_UNICODE_DIR)/emoji_table.rs \
	$(HARFRUST_SRC_DIR)/tag_table.rs \
	$(HARFRUST_SRC_DIR)/ot/shaper/vowel_constraints.rs

.PHONY: all rust hb-refresh clean

all: rust

rust: $(GENERATED)

# HarfBuzz's update-unicode-tables.make is path-sensitive and owns its own
# targets, so use it via a recursive make instead of including it directly.
hb-refresh:
	$(MAKE) -C $(HB_SRC) -f update-unicode-tables.make all

# Adapt imports emitted by the shared generators to HarfRust's module layout.
$(HARFRUST_UNICODE_DIR)/ucd_table.rs: $(HB_SRC)/gen-ucd-table.py $(HB_SRC)/ucd.nounihan.grouped.zip $(HB_SRC)/hb-script-list.h
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(word 2,$^) $(word 3,$^) > $@.tmp || ($(RM) $@.tmp; false)
	sed -e 's/crate::hb::algs/crate::algs/g' -e 's/_hb_ucd_/ucd_/g' -e 's/hb_script_t/Script/g' $@.tmp > $@
	$(RM) $@.tmp

$(HARFRUST_SRC_DIR)/ot/shaper/use_table.rs: $(HB_SRC)/gen-use-table.py $(HB_SRC)/IndicSyllabicCategory.txt $(HB_SRC)/IndicPositionalCategory.txt $(HB_SRC)/ArabicShaping.txt $(HB_SRC)/DerivedCoreProperties.txt $(HB_SRC)/UnicodeData.txt $(HB_SRC)/Blocks.txt $(HB_SRC)/Scripts.txt $(HB_SRC)/ms-use/IndicSyllabicCategory-Additional.txt $(HB_SRC)/ms-use/IndicPositionalCategory-Additional.txt
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,10,$^) > $@.tmp || ($(RM) $@.tmp; false)
	sed -e 's/super::shaper_use::/super::use_::/g' -e 's/hb_use_/use_/g' $@.tmp > $@
	$(RM) $@.tmp

$(HARFRUST_SRC_DIR)/ot/shaper/arabic_table.rs: $(HB_SRC)/gen-arabic-table.py $(HB_SRC)/ArabicShaping.txt $(HB_SRC)/UnicodeData.txt $(HB_SRC)/Blocks.txt
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,4,$^) > $@.tmp || ($(RM) $@.tmp; false)
	sed -e 's/super::shaper_arabic::/super::arabic::/g' -e 's/hb_arabic_joining_type_t/ArabicJoiningType/g' -e 's/_hb_arabic_joining_/arabic_joining_/g' $@.tmp > $@
	$(RM) $@.tmp

$(HARFRUST_SRC_DIR)/ot/shaper/arabic_pua.rs: $(HB_SRC)/gen-arabic-pua.py $(HB_SRC)/ArabicPUASimplified.txt $(HB_SRC)/ArabicPUATraditional.txt
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,3,$^) > $@.tmp || ($(RM) $@.tmp; false)
	sed 's/_hb_arabic_pua_/arabic_pua_/g' $@.tmp > $@
	$(RM) $@.tmp
	rustfmt $@

$(HARFRUST_SRC_DIR)/ot/shaper/indic_table.rs: $(HB_SRC)/gen-indic-table.py $(HB_SRC)/IndicSyllabicCategory.txt $(HB_SRC)/IndicPositionalCategory.txt $(HB_SRC)/Blocks.txt
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,4,$^) > $@.tmp || ($(RM) $@.tmp; false)
	sed -e 's/super::shaper_indic::/super::indic::/g' -e 's/ot_category_t/category/g' -e 's/ot_position_t/position/g' -e 's/_hb_indic_/indic_/g' $@.tmp > $@
	$(RM) $@.tmp

$(HARFRUST_UNICODE_DIR)/emoji_table.rs: $(HB_SRC)/gen-emoji-table.py $(HB_SRC)/emoji-data.txt $(HB_SRC)/emoji-test.txt
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$(PACKTAB_PYTHONPATH) \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,3,$^) > $@.tmp || ($(RM) $@.tmp; false)
	sed -e 's/crate::hb::unicode/crate::unicode/g' -e 's/_hb_emoji_/emoji_/g' -e 's/Extended_Pictographic/extended_pictographic/g' $@.tmp > $@
	$(RM) $@.tmp

$(HARFRUST_SRC_DIR)/tag_table.rs: $(HB_SRC)/gen-tag-table.py $(HB_SRC)/languagetags $(HB_SRC)/language-subtag-registry
	PYTHONDONTWRITEBYTECODE=1 \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,3,$^) > $@ || ($(RM) $@; false)

$(HARFRUST_SRC_DIR)/ot/shaper/vowel_constraints.rs: $(HB_SRC)/gen-vowel-constraints.py $(HB_SRC)/ms-use/IndicShapingInvalidCluster.txt $(HB_SRC)/Scripts.txt
	PYTHONDONTWRITEBYTECODE=1 \
		$(PYTHON) $(word 1,$^) --rust $(wordlist 2,3,$^) > $@.tmp || ($(RM) $@.tmp; false)
	sed -e 's/hb_buffer_t/Buffer/g' -e 's/super::buffer::/crate::buffer::/g' -e 's/super::script/crate::script/g' $@.tmp > $@
	$(RM) $@.tmp
	rustfmt $@

clean:
	$(RM) $(GENERATED)
