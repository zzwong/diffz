//! A real native text-backend probe, not a substitute for manual/automated reader interaction tests.
use crate::{native_text::NativeLine, theme::Skin};
use diffz_core::{
    domain::{SourceDocument, digest},
    layout::snap_grapheme,
};
use gpui_kit::{prelude::*, *};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
struct Probe {
    source: String,
    family: String,
    output: PathBuf,
    done: bool,
    failed: Arc<AtomicBool>,
    report: String,
}
fn measure(source: &str, family: &str, window: &mut Window) -> Value {
    let document = match SourceDocument::from_utf8(source.as_bytes().to_vec()) {
        Ok(d) => d,
        Err(e) => return json!({"status":"failed","error":e.to_string()}),
    };
    let line = (1..=document.lines().len() as u32)
        .filter_map(|i| document.line_text(i))
        .max_by_key(|s| s.len())
        .unwrap_or("");
    let mut cases = vec![];
    let mut passed = true;
    for font in [13.0, 18.0, 24.0] {
        for width in [300.0, 560.0, 900.0] {
            let started = Instant::now();
            let shaped = NativeLine::shape(
                line,
                width,
                font,
                (font * 1.65_f32).ceil(),
                true,
                family,
                window,
            );
            match shaped {
                Err(e) => {
                    passed = false;
                    cases.push(json!({"font":font,"width":width,"error":e.0}));
                }
                Ok(n) => {
                    let ranges: Vec<_> = n
                        .fragments
                        .iter()
                        .map(|f| [f.display.start, f.display.end])
                        .collect();
                    let complete = n.fragments.first().is_some_and(|f| f.display.start == 0)
                        && n.fragments
                            .last()
                            .is_some_and(|f| f.display.end == n.display.text.len())
                        && n.fragments
                            .windows(2)
                            .all(|p| p[0].display.end == p[1].display.start);
                    let point = snap_grapheme(line, line.len() / 2);
                    let fragment = n.fragment_for_source(point);
                    let source_point_valid =
                        n.source.is_char_boundary(n.source_at_fragment(fragment));
                    let last_y = n.fragments.last().map_or(0.0, |f| f.y) + n.line_height;
                    let fits_row = (last_y - n.height).abs() < 0.01;
                    let hits_valid = (0..n.fragments.len()).all(|i| {
                        n.hit(2.0, i as f32 * n.line_height + 1.0)
                            .is_some_and(|b| line.is_char_boundary(b))
                    });
                    passed &= complete && source_point_valid && fits_row && hits_valid;
                    cases.push(json!({"font":font,"width":width,"fragments":n.fragments.len(),"display_ranges":ranges,"native_height":n.height,"last_fragment_bottom":last_y,"raw_source_unchanged":n.source==line,"coverage_complete":complete,"source_anchor_byte":point,"anchor_fragment":fragment,"hit_bytes_valid":hits_valid,"fits_row":fits_row,"measurement_ms":started.elapsed().as_secs_f64()*1000.0}));
                }
            }
        }
    }
    let bidi_checks = bidi_checks(family, window);
    passed &= bidi_checks.iter().all(|case| case["passed"] == true);
    json!({"bidi_checks":bidi_checks,"schema_version":1,"status":if passed{"native_geometry_passed"}else{"failed"},"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"font_family":family,"input_digest":digest(&[source.as_bytes()]),"longest_source_line_bytes":line.len(),"cases":cases,"native_interaction_verified":false,"accessibility_verified":false,"note":"Actual GPUI/native-font measurement. This does NOT prove reader scrolling, pointer dispatch, clipboard, GPU frame latency or cross-platform correctness. Run docs/NATIVE_VALIDATION.md."})
}
// Small native-backend cases supplement the large fixtures with direction,
// caret, selection, mirroring and synthetic-control width assertions.
fn bidi_checks(family: &str, window: &mut Window) -> Vec<Value> {
    let mut checks = vec![];
    for (source, expected) in [("אבג", vec![4, 2, 0]), ("aאבz", vec![0, 3, 1, 5])] {
        let n = NativeLine::shape(source, 900.0, 18.0, 30.0, false, family, window).unwrap();
        let mut glyphs: Vec<_> = n.fragments[0]
            .layout
            .runs
            .iter()
            .flat_map(|r| &r.glyphs)
            .collect();
        glyphs.sort_by(|a, b| f32::from(a.position.x).total_cmp(&f32::from(b.position.x)));
        let indices: Vec<_> = glyphs.iter().map(|g| g.index).collect();
        let native = window.text_system().layout_line(
            source,
            px(18.0),
            &[crate::native_text::run(
                source.len(),
                family,
                rgb(0xffffff).into(),
            )],
            None,
        );
        let width_matches = (f32::from(native.width) - n.width).abs() < 0.1;
        let caret = if source == "אבג" {
            n.hit(0.1, 1.0) == Some(source.len())
        } else {
            n.rectangles(0..3).len() == 2
        };
        checks.push(json!({"source":source,"visual_indices":indices,"width_matches_native":width_matches,"passed":indices == expected && width_matches && caret}));
    }
    let n = NativeLine::shape("(אב)", 900.0, 18.0, 30.0, false, family, window).unwrap();
    let native = window.text_system().layout_line(
        "(",
        px(18.0),
        &[crate::native_text::run(1, family, rgb(0xffffff).into())],
        None,
    );
    let left = n.fragments[0]
        .layout
        .runs
        .iter()
        .flat_map(|r| &r.glyphs)
        .min_by(|a, b| f32::from(a.position.x).total_cmp(&f32::from(b.position.x)))
        .unwrap();
    checks.push(
        json!({"source":"(אב)","passed":left.index == 5 && left.id == native.runs[0].glyphs[0].id}),
    );
    checks
}
impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.done {
            self.done = true;
            let report = measure(&self.source, &self.family, window);
            let ok = report["status"] == "native_geometry_passed";
            let save = (|| -> std::io::Result<()> {
                let mut opts = std::fs::OpenOptions::new();
                opts.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    opts.mode(0o600);
                }
                let mut file = opts.open(&self.output)?;
                let encoded =
                    serde_json::to_string_pretty(&report).map_err(std::io::Error::other)?;
                file.write_all(encoded.as_bytes())?;
                file.write_all(b"\n")?;
                file.sync_all()
            })();
            self.failed.store(!ok || save.is_err(), Ordering::Relaxed);
            self.report = match save {
                Ok(()) => format!(
                    "{} · evidence written to {}",
                    report["status"],
                    self.output.display()
                ),
                Err(e) => format!("evidence write failed: {e}"),
            };
            // Allow a frame before terminating; the report explicitly excludes interaction/paint proof.
            cx.spawn(async move |this, cx| {
                smol::Timer::after(Duration::from_millis(500)).await;
                let _ = this.update(cx, |probe, cx| {
                    // macOS terminate does not return from Application::run.
                    if probe.failed.load(Ordering::Relaxed) {
                        std::process::exit(2);
                    }
                    cx.quit();
                });
            })
            .detach();
        }
        div()
            .size_full()
            .p_5()
            .bg(Skin::new(true).base)
            .text_color(Skin::new(true).text)
            .child(self.report.clone())
    }
}
pub fn launch_probe(source: String, font: Option<String>, output: PathBuf) {
    let failed = Arc::new(AtomicBool::new(true));
    let result = failed.clone();
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let family = font.unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "Menlo".into()
            } else {
                "DejaVu Sans Mono".into()
            }
        });
        cx.spawn(async move |cx| {
            let opened = cx.open_window(WindowOptions::default(), |_, cx| {
                cx.new(|_| Probe {
                    source,
                    family,
                    output,
                    done: false,
                    failed,
                    report: String::new(),
                })
            });
            if let Err(e) = opened {
                eprintln!("native probe window failed: {e}");
                std::process::exit(2);
            }
        })
        .detach();
    });
    if result.load(Ordering::Relaxed) {
        std::process::exit(2)
    }
}
