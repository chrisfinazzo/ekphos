use std::borrow::Cow;
use std::cell::Cell;
use std::sync::{Arc, Once, OnceLock};

use image::{DynamicImage, RgbaImage};
use resvg::{tiny_skia, usvg};

pub const MAX_DIAGRAM_SOURCE_BYTES: usize = 64 * 1024;
pub const SVG_UNITS_PER_ROW: f32 = 16.8;
const RENDERER_VERSION: &str = "mermaid-svg-0.7.0-v1";
const PAPER_PADDING: f32 = 16.0;
const PREFERRED_SANS_FAMILIES: [&str; 12] = ["Arial", "Helvetica", "Liberation Sans", "Arimo", "Helvetica Neue", "Inter", "Segoe UI", "Roboto", "Noto Sans", "DejaVu Sans", "Ubuntu", "Cantarell"];
const FALLBACK_FONT: (&str, &str) = ("KaTeX_SansSerif-Regular.ttf", "KaTeX_SansSerif");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiagramPalette {
    pub text: [u8; 3],
    pub muted: [u8; 3],
    pub background: [u8; 3],
    pub accent: [u8; 3],
    pub secondary: [u8; 3],
    pub info: [u8; 3],
    pub success: [u8; 3],
    pub warning: [u8; 3],
    pub error: [u8; 3],
    pub border: [u8; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagramStyle {
    Themed(DiagramPalette),
    Light,
    Dark,
}

pub fn diagram_key(source: &str, style: &DiagramStyle) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    RENDERER_VERSION.hash(&mut hasher);
    source.hash(&mut hasher);
    style.hash(&mut hasher);
    format!("diagram:{:016x}", hasher.finish())
}

pub fn diagram_kind(source: &str) -> &'static str {
    let mut in_frontmatter = false;
    for line in source.lines().map(str::trim) {
        if line == "---" {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if in_frontmatter || line.is_empty() || line.starts_with("%%") {
            continue;
        }
        let keyword = line.split(|character: char| character.is_whitespace() || character == ';').next().unwrap_or("").to_ascii_lowercase();
        return match keyword.as_str() {
            "graph" | "flowchart" | "flowchart-elk" => "Flowchart",
            "sequencediagram" => "Sequence diagram",
            "classdiagram" | "classdiagram-v2" => "Class diagram",
            "statediagram" | "statediagram-v2" => "State diagram",
            "erdiagram" => "ER diagram",
            "journey" => "User journey",
            "gantt" => "Gantt chart",
            "pie" => "Pie chart",
            "quadrantchart" => "Quadrant chart",
            "requirementdiagram" => "Requirement diagram",
            "gitgraph" => "Git graph",
            "c4context" | "c4container" | "c4component" | "c4dynamic" | "c4deployment" => "C4 diagram",
            "mindmap" => "Mindmap",
            "timeline" => "Timeline",
            "zenuml" => "ZenUML",
            "sankey" | "sankey-beta" => "Sankey diagram",
            "xychart" | "xychart-beta" => "XY chart",
            "block" | "block-beta" => "Block diagram",
            "packet" | "packet-beta" => "Packet diagram",
            "kanban" => "Kanban",
            "architecture" | "architecture-beta" => "Architecture diagram",
            "radar" | "radar-beta" => "Radar chart",
            "treemap" | "treemap-beta" => "Treemap",
            _ => "Diagram",
        };
    }
    "Diagram"
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiagramView {
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub scale: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}

impl DiagramView {
    pub fn fit(width: f32, height: f32, pixel_width: u32, pixel_height: u32) -> Self {
        let scale = (pixel_width as f32 / width.max(1.0)).min(pixel_height as f32 / height.max(1.0));
        Self { pixel_width, pixel_height, scale, origin_x: width / 2.0 - pixel_width as f32 / scale / 2.0, origin_y: height / 2.0 - pixel_height as f32 / scale / 2.0 }
    }
}

pub struct DiagramScene {
    tree: usvg::Tree,
    backdrop: Option<[u8; 3]>,
    bytes: usize,
}

impl DiagramScene {
    pub fn render(source: &str, style: &DiagramStyle) -> Result<Self, String> {
        let source = source.trim();
        if source.is_empty() {
            return Err("empty diagram".to_string());
        }
        if source.len() > MAX_DIAGRAM_SOURCE_BYTES {
            return Err("diagram exceeds the 64 KB source limit".to_string());
        }
        let (svg, backdrop) = silence_panics(|| {
            let declared = mermaid_svg::parse_with_meta(source).map_err(|error| error.to_string())?.1.theme;
            let (theme, backdrop) = match (declared.as_deref().and_then(mermaid_svg::Theme::by_name), style) {
                (Some(theme), _) => {
                    let backdrop = parse_hex(&theme.bg).unwrap_or([255, 255, 255]);
                    (theme, Some(backdrop))
                }
                (None, DiagramStyle::Themed(palette)) => (themed(palette), None),
                (None, DiagramStyle::Light) => (mermaid_svg::Theme::default_theme(), Some([255, 255, 255])),
                (None, DiagramStyle::Dark) => {
                    let theme = mermaid_svg::Theme::dark();
                    let backdrop = parse_hex(&theme.bg).unwrap_or([30, 30, 30]);
                    (theme, Some(backdrop))
                }
            };
            let theme = mermaid_svg::Theme { responsive: false, ..theme };
            mermaid_svg::render_with(source, &theme).map(|svg| (legible_on_dark(svg, &theme), backdrop)).map_err(|error| error.to_string())
        })?;
        let options = usvg::Options { fontdb: font_database(), ..usvg::Options::default() };
        let tree = usvg::Tree::from_str(&svg, &options).map_err(|error| format!("invalid diagram output: {error}"))?;
        Ok(Self { tree, backdrop, bytes: svg.len().saturating_mul(4).saturating_add(4096) })
    }

    pub fn width(&self) -> f32 {
        self.tree.size().width()
    }

    pub fn height(&self) -> f32 {
        self.tree.size().height()
    }

    pub fn backdrop(&self) -> Option<[u8; 3]> {
        self.backdrop
    }

    pub fn estimated_bytes(&self) -> usize {
        self.bytes
    }

    pub fn rasterize(&self, view: DiagramView, fill: Option<[u8; 3]>) -> Result<DynamicImage, String> {
        let pixels = u64::from(view.pixel_width) * u64::from(view.pixel_height);
        if pixels == 0 || view.pixel_width > crate::image_service::MAX_IMAGE_DIMENSION || view.pixel_height > crate::image_service::MAX_IMAGE_DIMENSION || pixels * 4 > crate::image_service::MAX_DECODED_IMAGE_BYTES as u64 {
            return Err("diagram view exceeds image limits".to_string());
        }
        if !view.scale.is_finite() || view.scale <= 0.0 {
            return Err("diagram view has an invalid scale".to_string());
        }
        let mut pixmap = tiny_skia::Pixmap::new(view.pixel_width, view.pixel_height).ok_or_else(|| "diagram pixmap allocation failed".to_string())?;
        if let Some([red, green, blue]) = fill {
            pixmap.fill(tiny_skia::Color::from_rgba8(red, green, blue, 255));
        }
        let transform = tiny_skia::Transform::from_row(view.scale, 0.0, 0.0, view.scale, -view.origin_x * view.scale, -view.origin_y * view.scale);
        if let (Some([red, green, blue]), Some(paper)) = (self.backdrop, tiny_skia::Rect::from_xywh(-PAPER_PADDING, -PAPER_PADDING, self.width() + 2.0 * PAPER_PADDING, self.height() + 2.0 * PAPER_PADDING)) {
            let mut paint = tiny_skia::Paint::default();
            paint.set_color_rgba8(red, green, blue, 255);
            pixmap.fill_rect(paper, &paint, transform, None);
        }
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        let mut rgba = Vec::with_capacity(pixmap.data().len());
        for pixel in pixmap.pixels() {
            let color = pixel.demultiply();
            rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
        }
        RgbaImage::from_raw(view.pixel_width, view.pixel_height, rgba).map(DynamicImage::ImageRgba8).ok_or_else(|| "diagram raster has invalid dimensions".to_string())
    }
}

thread_local! {
    static PANICS_SILENCED: Cell<bool> = const { Cell::new(false) };
}

fn silence_panics<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !PANICS_SILENCED.with(Cell::get) {
                previous(info);
            }
        }));
    });
    PANICS_SILENCED.with(|silenced| silenced.set(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
    PANICS_SILENCED.with(|silenced| silenced.set(false));
    result.unwrap_or_else(|_| Err("unsupported or malformed diagram syntax".to_string()))
}

fn font_database() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    Arc::clone(FONTS.get_or_init(|| {
        let mut database = usvg::fontdb::Database::new();
        database.load_system_fonts();
        let has_family = |database: &usvg::fontdb::Database, family: &str| database.faces().any(|face| face.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family)));
        let family = PREFERRED_SANS_FAMILIES.iter().find(|family| has_family(&database, family)).map(|family| family.to_string()).or_else(|| database.faces().find_map(|face| face.families.first().map(|(name, _)| name.clone())));
        let family = family.unwrap_or_else(|| {
            if let Some(bytes) = ratex_katex_fonts::ttf_bytes(FALLBACK_FONT.0) {
                database.load_font_data(bytes.into_owned());
            }
            FALLBACK_FONT.1.to_string()
        });
        database.set_sans_serif_family(family);
        Arc::new(database)
    }))
}

fn themed(palette: &DiagramPalette) -> mermaid_svg::Theme {
    let dark = luminance(palette.background) < 0.5;
    let background = palette.background;
    let node_fill = mix(background, palette.accent, if dark { 0.18 } else { 0.10 });
    let series = [palette.accent, palette.info, palette.success, palette.warning, palette.secondary, palette.error];
    let swatches: Vec<Cow<'static, str>> = (0..12).map(|index| hex(mix(background, series[index % series.len()], if index < series.len() { 0.75 } else { 0.45 }))).collect();
    let quadrants = [0.06, 0.10, 0.14, 0.18].map(|amount| hex(mix(background, palette.accent, amount)));
    mermaid_svg::Theme {
        fg: hex(palette.text),
        fg_muted: hex(palette.muted),
        bg: hex(background),
        actor_fill: hex(node_fill),
        actor_stroke: hex(palette.accent),
        actor_text_color: Some(hex(palette.text)),
        lifeline: hex(mix(background, palette.muted, 0.7)),
        arrow_stroke: hex(palette.text),
        signal_text_color: Some(hex(palette.text)),
        note_fill: hex(mix(background, palette.warning, if dark { 0.20 } else { 0.18 })),
        note_stroke: hex(mix(background, palette.warning, 0.8)),
        activation_fill: hex(mix(background, palette.accent, 0.28)),
        activation_stroke: hex(palette.accent),
        frame_label_fill: hex(node_fill),
        title_color: Some(hex(palette.text)),
        flow_node_fill: hex(node_fill),
        flow_node_stroke: hex(palette.accent),
        flow_edge_stroke: hex(mix(background, palette.text, 0.75)),
        flow_label_bg: hex(mix(background, palette.text, 0.10)),
        flow_cluster_fill: hex(mix(background, palette.secondary, 0.07)),
        flow_cluster_stroke: hex(mix(background, palette.secondary, 0.6)),
        cscale_palette: Cow::Owned(swatches.clone()),
        pie_palette: Cow::Owned(swatches.clone()),
        git_palette: Cow::Owned(swatches.clone()),
        xychart_palette: Cow::Owned(swatches),
        pie_stroke: Some(hex(background)),
        pie_opacity: None,
        commit_label_color: Some(hex(palette.text)),
        tag_label_color: Some(hex(palette.text)),
        quadrant_fills: [None, None, None, None],
        quadrant_default_fills: quadrants,
        font_family: Cow::Borrowed("sans-serif"),
        font_size: 14.0,
        responsive: false,
    }
}

fn legible_on_dark(svg: String, theme: &mermaid_svg::Theme) -> String {
    if parse_hex(&theme.fg).is_none_or(|text| luminance(text) < 0.5) {
        return svg;
    }
    let surface = format!("fill=\"{}\"", theme.flow_node_fill);
    svg.replace("fill=\"#efefef\"", &surface).replace("fill=\"#e8e8e8\"", &surface).replace("stroke=\"black\"", &format!("stroke=\"{}\"", theme.fg_muted))
}

fn hex([red, green, blue]: [u8; 3]) -> Cow<'static, str> {
    Cow::Owned(format!("#{red:02x}{green:02x}{blue:02x}"))
}

fn mix(from: [u8; 3], to: [u8; 3], amount: f32) -> [u8; 3] {
    let amount = amount.clamp(0.0, 1.0);
    [0, 1, 2].map(|channel| (f32::from(from[channel]) + (f32::from(to[channel]) - f32::from(from[channel])) * amount).round() as u8)
}

fn luminance([red, green, blue]: [u8; 3]) -> f32 {
    (0.2126 * f32::from(red) + 0.7152 * f32::from(green) + 0.0722 * f32::from(blue)) / 255.0
}

fn parse_hex(value: &str) -> Option<[u8; 3]> {
    let digits = value.trim().strip_prefix('#')?;
    let channel = |text: &str| u8::from_str_radix(text, 16).ok();
    match digits.len() {
        3 => {
            let mut channels = digits.chars().map(|digit| channel(&digit.to_string()).map(|value| value * 17));
            Some([channels.next()??, channels.next()??, channels.next()??])
        }
        6 => Some([channel(&digits[0..2])?, channel(&digits[2..4])?, channel(&digits[4..6])?]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PALETTE: DiagramPalette =
        DiagramPalette { text: [220, 220, 220], muted: [140, 140, 140], background: [24, 24, 28], accent: [130, 170, 255], secondary: [190, 140, 255], info: [100, 200, 230], success: [120, 220, 140], warning: [240, 200, 100], error: [240, 110, 110], border: [70, 70, 80] };

    #[test]
    fn flowchart_renders_to_a_scene_with_positive_size() {
        let scene = DiagramScene::render("flowchart TD\n  A[Start] --> B{Ok?}\n  B -->|Yes| C[Done]", &DiagramStyle::Themed(PALETTE)).unwrap();
        assert!(scene.width() > 10.0 && scene.height() > 10.0);
        assert!(scene.backdrop().is_none());
        let image = scene.rasterize(DiagramView::fit(scene.width(), scene.height(), 200, 300), None).unwrap();
        assert_eq!((image.width(), image.height()), (200, 300));
        assert!(image.to_rgba8().pixels().any(|pixel| pixel[3] > 0));
    }

    #[test]
    fn welcome_note_diagrams_render() {
        let mut count = 0;
        for note in [include_str!("../assets/welcome/getting-started.md"), include_str!("../assets/welcome/demo-note.md")] {
            for block in note.split("```mermaid\n").skip(1) {
                let source = block.split("\n```").next().unwrap();
                DiagramScene::render(source, &DiagramStyle::Themed(PALETTE)).unwrap();
                count += 1;
            }
        }
        assert_eq!(count, 2);
    }

    #[test]
    fn invalid_and_panicking_sources_become_errors() {
        assert!(DiagramScene::render("not a diagram at all", &DiagramStyle::Light).is_err());
        assert!(DiagramScene::render("architecture-beta\nservice]a(x)[y  in z", &DiagramStyle::Light).is_err());
        assert!(DiagramScene::render("   ", &DiagramStyle::Light).is_err());
    }

    #[test]
    fn declared_themes_and_light_style_paint_a_paper_backdrop() {
        let declared = DiagramScene::render("%%{init: {\"theme\": \"forest\"}}%%\ngraph LR\n  A --> B", &DiagramStyle::Themed(PALETTE)).unwrap();
        assert_eq!(declared.backdrop(), Some([240, 248, 240]));
        let light = DiagramScene::render("graph LR\n  A --> B", &DiagramStyle::Light).unwrap();
        assert_eq!(light.backdrop(), Some([255, 255, 255]));
        let image = light.rasterize(DiagramView::fit(light.width(), light.height(), 100, 100), None).unwrap().to_rgba8();
        assert_eq!(image.get_pixel(50, 50)[3], 255);
    }

    #[test]
    fn keys_change_with_source_and_style() {
        let themed = diagram_key("graph LR\nA-->B", &DiagramStyle::Themed(PALETTE));
        assert_eq!(themed, diagram_key("graph LR\nA-->B", &DiagramStyle::Themed(PALETTE)));
        assert_ne!(themed, diagram_key("graph LR\nA-->C", &DiagramStyle::Themed(PALETTE)));
        assert_ne!(themed, diagram_key("graph LR\nA-->B", &DiagramStyle::Light));
    }

    #[test]
    fn kinds_skip_directives_and_frontmatter() {
        assert_eq!(diagram_kind("%%{init: {}}%%\n\nsequenceDiagram\n  A->>B: hi"), "Sequence diagram");
        assert_eq!(diagram_kind("---\ntitle: Flow\n---\nflowchart LR\n A-->B"), "Flowchart");
        assert_eq!(diagram_kind("graph TD;A-->B"), "Flowchart");
        assert_eq!(diagram_kind("somethingElse"), "Diagram");
    }

    #[test]
    fn fit_view_centers_the_scene() {
        let view = DiagramView::fit(100.0, 50.0, 400, 400);
        assert!((view.scale - 4.0).abs() < f32::EPSILON);
        assert!((view.origin_x - 0.0).abs() < 0.001);
        assert!((view.origin_y - -25.0).abs() < 0.001);
    }

    #[test]
    fn hardcoded_light_fills_follow_dark_themes_only() {
        let svg = "<rect fill=\"#efefef\" stroke=\"black\"/><rect fill=\"#e8e8e8\"/>".to_string();
        let dark = legible_on_dark(svg.clone(), &themed(&PALETTE));
        assert!(!dark.contains("#efefef") && !dark.contains("#e8e8e8") && !dark.contains("black"), "{dark}");
        assert_eq!(legible_on_dark(svg.clone(), &mermaid_svg::Theme::default_theme()), svg);
    }

    #[test]
    fn hex_colors_parse_short_and_long_forms() {
        assert_eq!(parse_hex("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("#1E1E1E"), Some([30, 30, 30]));
        assert_eq!(parse_hex("white"), None);
    }
}
