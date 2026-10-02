use criterion::{criterion_group, criterion_main, Criterion};
use std::path::Path;

const BENCHES: &[(&str, &str)] = &[
    (
        "benches/fonts/Roboto-Regular.ttf",
        "benches/texts/en-thelittleprince.txt",
    ),
    (
        "benches/fonts/Roboto-Regular.ttf",
        "benches/texts/en-words.txt",
    ),
    (
        "benches/fonts/NotoNastaliqUrdu-Regular.ttf",
        "benches/texts/fa-thelittleprince.txt",
    ),
    (
        "benches/fonts/NotoNastaliqUrdu-Regular.ttf",
        "benches/texts/fa-words.txt",
    ),
    (
        "benches/fonts/NotoSansDevanagari-Regular.ttf",
        "benches/texts/hi-words.txt",
    ),
    (
        "benches/fonts/Amiri-Regular.ttf",
        "benches/texts/fa-thelittleprince.txt",
    ),
    (
        "benches/fonts/SourceSerifVariable-Roman.ttf",
        "benches/texts/react-dom.txt",
    ),
];

#[derive(Default)]
struct ShapePlanCache {
    plans: Vec<harfrust::ShapePlan>,
}

impl ShapePlanCache {
    fn get(
        &mut self,
        instance: &harfrust::font::Font,
        buffer: &harfrust::Buffer,
    ) -> &harfrust::ShapePlan {
        let key = harfrust::ShapePlanKey::new(instance, buffer.script(), buffer.direction());
        if let Some(plan_idx) = self.plans.iter().position(|plan| key.matches(plan)) {
            &self.plans[plan_idx]
        } else {
            self.plans.push(harfrust::ShapePlan::new(
                instance,
                buffer.direction(),
                buffer.script(),
                None,
                &[],
            ));
            self.plans.last().unwrap()
        }
    }
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("shaping");
    group.sampling_mode(criterion::SamplingMode::Flat);
    for (font_path, text_path) in BENCHES {
        let font_path: &Path = font_path.as_ref();
        let text_path: &Path = text_path.as_ref();
        let font_data = std::fs::read(font_path).unwrap();
        let text = std::fs::read_to_string(text_path).unwrap();
        let lines = text.trim().lines().collect::<Vec<_>>();
        let mut test_name = font_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        test_name.push('/');
        test_name.push_str(&text_path.file_name().unwrap().to_string_lossy());
        group.bench_function(&(test_name.clone() + "/hr"), |b| {
            let font = harfrust::font::Font::new(font_data.clone(), 0).unwrap();
            let instance = font.instance_builder().build();
            let shaping_font = harfrust::ShaperFont::new(&instance);
            let mut plan_cache = ShapePlanCache::default();
            let mut buffer = harfrust::Buffer::new();
            b.iter(|| {
                for line in &lines {
                    buffer.clear();
                    buffer.push_str(line);
                    buffer.guess_segment_properties();
                    let plan = plan_cache.get(&instance, &buffer);
                    harfrust::shape(
                        &shaping_font,
                        &mut buffer,
                        harfrust::ShapeOptions::new().plan(Some(plan)),
                    )
                    .unwrap();
                }
            });
        });
        group.bench_function(&(test_name + "/hb"), |b| {
            let face = harfbuzz_rs::Face::from_bytes(&font_data, 0);
            let font = harfbuzz_rs::Font::new(face);
            let mut shared_buffer = Some(harfbuzz_rs::UnicodeBuffer::new());
            b.iter(|| {
                for line in &lines {
                    let buffer = shared_buffer
                        .take()
                        .unwrap()
                        .add_str(line)
                        .guess_segment_properties();
                    shared_buffer = Some(harfbuzz_rs::shape(&font, buffer, &[]).clear());
                }
            });
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(100))
        .measurement_time(std::time::Duration::from_millis(500))
        .sample_size(10);
    targets = bench
}
criterion_main!(benches);
