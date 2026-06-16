use harfrust::{
    font::{Font, FontInstance, FontTableFunction},
    shape, SerializeFlags, ShapeOptions, UnicodeBuffer,
};
use objc2_core_foundation::{self as cf, CFData, CFRetained};
use objc2_core_text as ct;
use std::{ops::Deref, sync::Arc};

#[test]
fn macos_compare_lucida_grande() {
    compare_data_and_ct(
        "/System/Library/Fonts/LucidaGrande.ttc",
        "Lucida Grande",
        "\u{20DD}\u{1F174}\u{1F175}",
        "[circlecmb=0+0|.notdef=1+1536|.notdef=2+1536]",
    );
}

#[test]
fn macos_compare_geeza_pro() {
    compare_data_and_ct(
    "/System/Library/Fonts/GeezaPro.ttc",
    "Geeza Pro",
    "\u{0628}\u{064A}\u{064E}\u{0651}\u{0629}",
    "[u0629.final.tehMarbuta=4+713|u064e_u0651.shaddaFatha=1@0,-200+0|u064a.medial.yeh=1+656|u0628.initial.beh=0+656]"
    );
}

#[test]
fn macos_compare_big_caslon() {
    compare_data_and_ct(
        "/System/Library/Fonts/Supplemental/BigCaslon.ttf",
        "Big Caslon",
        "\u{0107}",
        "[cacute=0+432]",
    );
}

#[test]
fn macos_compare_helvetica() {
    compare_data_and_ct(
        "/System/Library/Fonts/Helvetica.ttc",
        "Helvetica",
        "\u{006D}\u{0300}",
        "[m=0+1706|gravecmb=0@-284,10+0]",
    );
}

#[test]
fn macos_compare_apple_chancery() {
    compare_data_and_ct(
        "/System/Library/Fonts/Supplemental/Apple Chancery.ttf",
        "Apple Chancery",
        "\u{0066}\u{0069}\u{0072}\u{0073}\u{0074}",
        "[f_i=0+1097|r=2+853|s=3+728|t=4+725]",
    );
}

#[test]
fn macos_compare_kokonor() {
    compare_data_and_ct(
        "/System/Library/Fonts/Supplemental/Kokonor.ttf",
        "Kokonor",
        "\u{0F62}\u{0F92}\u{0FB1}\u{0F74}",
        "[r_g_y_u=0+1579]",
    );
}

#[test]
fn macos_compare_bangla() {
    compare_data_and_ct(
        "/System/Library/Fonts/Supplemental/Bangla MN.ttc",
        "Bangla MN",
        "\u{09AC}\u{09BF}",
        "[bn_ikaar=0+474|bn_ba=0+998]",
    );
}

#[track_caller]
fn compare_data_and_ct(font_path: &str, font_name: &str, input: &str, expected_output: &str) {
    let font_data = std::fs::read(font_path).unwrap();
    let data_instance = instance_for_font_data(font_data);
    let ct_font = unsafe {
        ct::CTFont::with_name(&cf::CFString::from_str(font_name), 16.0, std::ptr::null())
    };
    let ct_instance = instance_for_ct_font(ct_font);
    let [data_out, ct_out] = [data_instance, ct_instance].map(|instance| {
        let mut buffer = UnicodeBuffer::new();
        for (i, ch) in input.chars().enumerate() {
            buffer.add(ch, i as u32);
        }
        buffer.guess_segment_properties();
        let glyphs = shape(&instance, buffer, ShapeOptions::default());
        glyphs.serialize(&instance, SerializeFlags::default())
    });
    assert_eq!(data_out, ct_out);
    assert_eq!(data_out, expected_output);
}

fn instance_for_font_data(data: Vec<u8>) -> FontInstance {
    let font = Font::new(data, 0).unwrap();
    FontInstance::builder(&font).build()
}

fn instance_for_ct_font(ct_font: CFRetained<ct::CTFont>) -> FontInstance {
    let ct_font = CTFontWrapper(ct_font);
    let table_fn = unsafe {
        FontTableFunction::new(Arc::new(move |tag| {
            let table_data = ct_font.table(
                u32::from_be_bytes(tag.to_be_bytes()),
                ct::CTFontTableOptions::NoOptions,
            )?;
            Some(harfrust::font::FontBlob::Shared(Arc::new(CFDataWrapper(
                table_data,
            ))))
        }))
    };
    let font = Font::new(table_fn, 0).unwrap();
    FontInstance::builder(&font).build()
}

struct CTFontWrapper(CFRetained<ct::CTFont>);

unsafe impl Send for CTFontWrapper {}
unsafe impl Sync for CTFontWrapper {}

impl Deref for CTFontWrapper {
    type Target = CFRetained<ct::CTFont>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

struct CFDataWrapper(CFRetained<CFData>);

impl AsRef<[u8]> for CFDataWrapper {
    fn as_ref(&self) -> &[u8] {
        unsafe { self.0.as_bytes_unchecked() }
    }
}

unsafe impl Send for CFDataWrapper {}
unsafe impl Sync for CFDataWrapper {}
