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
        shaper: &harfrust::Shaper,
        buffer: &harfrust::UnicodeBuffer,
    ) -> &harfrust::ShapePlan {
        let key = harfrust::ShapePlanKey::new(Some(buffer.script()), buffer.direction());
        if let Some(plan_idx) = self.plans.iter().position(|plan| key.matches(plan)) {
            &self.plans[plan_idx]
        } else {
            self.plans.push(harfrust::ShapePlan::new(
                shaper,
                buffer.direction(),
                Some(buffer.script()),
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
            let font = harfrust::FontRef::from_index(&font_data, 0).unwrap();
            let state = HrTestState::new(&font);
            let shaper = state.shaper();
            let mut plan_cache = ShapePlanCache::default();
            let mut shared_buffer = Some(harfrust::UnicodeBuffer::new());
            b.iter(|| {
                for line in &lines {
                    let mut buffer = shared_buffer.take().unwrap();
                    buffer.push_str(line);
                    buffer.guess_segment_properties();
                    let plan = plan_cache.get(&shaper, &buffer);
                    shared_buffer = Some(shaper.shape_with_plan(plan, buffer, &[]).clear());
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

struct HrTestState<'a> {
    font: &'a harfrust::FontRef<'a>,
    data: harfrust::ShaperData,
    _instance: Option<harfrust::ShaperInstance>,
}

impl<'a> HrTestState<'a> {
    fn new(font: &'a harfrust::FontRef<'a>) -> Self {
        let data = harfrust::ShaperData::new(font);
        Self {
            font,
            data,
            _instance: None,
        }
    }

    fn shaper(&self) -> harfrust::Shaper<'_> {
        self.data
            .shaper(self.font)
            .instance(self._instance.as_ref())
            .build()
    }
}
