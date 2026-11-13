use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    /// Original id from an imported file (Figma "12:34"), for round-tripping.
    pub source_id: Option<String>,
    pub name: String,
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    /// Back-to-front order.
    pub children: Vec<NodeId>,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    /// Transform relative to the parent (Figma `relativeTransform`).
    pub transform: Affine,
    /// Untransformed size.
    pub size: Vec2,
    pub fills: Vec<Paint>,
    pub stroke: Stroke,
    pub effects: Vec<Effect>,
    /// Corner radii (top-left, top-right, bottom-right, bottom-left).
    pub corner_radii: [f32; 4],
    /// iOS-style corner smoothing 0..1.
    pub corner_smoothing: f32,
    pub constraints: Constraints,
    /// Child behaviour inside an auto-layout parent.
    pub layout_child: LayoutChild,
    pub is_mask: bool,
    /// Style id per property ("fill", "stroke", "text", "effect", "grid").
    pub style_refs: BTreeMap<String, String>,
    /// Precomputed fill outline in local space (Figma `fillGeometry`). When
    /// present, renderers may use it instead of deriving geometry, e.g. for
    /// boolean operations whose result was computed by Figma.
    #[serde(default)]
    pub fill_geometry: Option<Vec<Path>>,
    /// Precomputed stroke outline in local space (Figma `strokeGeometry`),
    /// already expanded by weight/align; filled with the stroke paints.
    #[serde(default)]
    pub stroke_geometry: Option<Vec<Path>>,
    /// Component/instance bookkeeping; empty for ordinary nodes.
    #[serde(default, skip_serializing_if = "ComponentLink::is_empty")]
    pub component: ComponentLink,
    /// Prototype interactions and scroll behaviour; empty for most nodes.
    #[serde(default, skip_serializing_if = "PrototypeProps::is_empty")]
    pub prototype: PrototypeProps,
    /// Variable bindings and explicit modes; empty for most nodes.
    #[serde(default, skip_serializing_if = "NodeVariables::is_empty")]
    pub variables: NodeVariables,
    /// Export presets shown in the inspector's Export section.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub export_settings: Vec<ExportPreset>,
    /// Dev mode status, links and annotations; empty for most nodes.
    #[serde(default, skip_serializing_if = "DevInfo::is_empty")]
    pub dev: DevInfo,
    /// Presentation metadata; ordinary design nodes omit this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slide: Option<SlideProps>,
    /// How a board drawing stroke (marker, highlighter, washi tape) is drawn; `None` for
    /// everything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<Brush>,
    /// The pictures AI generations gave this layer, oldest first, so an earlier one can be
    /// brought back; empty for everything else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ai_versions: Vec<AiVersion>,
    /// Motion keyframe tracks and clips; empty for nodes that are not animated.
    #[serde(default, skip_serializing_if = "NodeMotion::is_empty")]
    pub motion: NodeMotion,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlideProps {
    #[serde(default)]
    pub notes: String,
}

/// A node property that an instance can override independently of its main
/// component. Syncing a component into its instances copies every field that
/// is not in the instance node's [`ComponentLink::overrides`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OverrideField {
    Name,
    Visible,
    Locked,
    Opacity,
    Blend,
    Size,
    Transform,
    Fills,
    Stroke,
    Effects,
    CornerRadii,
    /// Text characters.
    Characters,
    /// Text style and runs.
    TextStyle,
    /// Frame props: clipping, auto layout, grids.
    Layout,
    /// Which component an instance points at (instance swap).
    MainComponent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PropValue {
    Bool(bool),
    Text(String),
    /// Selected option of a variant property.
    Variant(String),
    /// Component swapped in, by node id.
    Instance(NodeId),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PropKind {
    Boolean,
    Text,
    Variant { options: Vec<String> },
    InstanceSwap { preferred: Vec<NodeId> },
}

/// Component property definition, on a Component or ComponentSet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentPropDef {
    pub name: String,
    pub kind: PropKind,
    pub default: PropValue,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComponentLink {
    /// For a node inside an instance: the node of the main component it mirrors.
    /// Instance roots point at their component via `NodeKind::Instance::component`.
    pub source: Option<NodeId>,
    /// Properties changed on this instance node; preserved when syncing.
    pub overrides: BTreeSet<OverrideField>,
    /// Property definitions (Component / ComponentSet only).
    pub props: Vec<ComponentPropDef>,
    /// Property values chosen on an instance root, by property name.
    pub values: BTreeMap<String, PropValue>,
    /// Binds a node property to a component property, e.g.
    /// `Visible -> "Show icon"`, `Characters -> "Label"`, `MainComponent -> "Icon"`.
    pub bindings: BTreeMap<OverrideField, String>,
}

impl ComponentLink {
    pub fn is_empty(&self) -> bool {
        self.source.is_none() && self.overrides.is_empty() && self.props.is_empty() && self.values.is_empty() && self.bindings.is_empty()
    }
}

/// One picture in a layer's AI history: the image it showed and the prompt that made it
/// (empty for the picture the layer had before the first generation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiVersion {
    pub image_ref: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
}

impl Node {
    pub fn new(id: NodeId, kind: NodeKind) -> Self {
        Self {
            id,
            source_id: None,
            name: kind.default_name().into(),
            kind,
            parent: None,
            children: Vec::new(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: BlendMode::PassThrough,
            transform: Affine::IDENTITY,
            size: Vec2::ZERO,
            fills: Vec::new(),
            stroke: Stroke { weight: 1.0, miter_limit: 4.0, ..Default::default() },
            effects: Vec::new(),
            corner_radii: [0.0; 4],
            corner_smoothing: 0.0,
            constraints: Constraints::default(),
            layout_child: LayoutChild::default(),
            is_mask: false,
            style_refs: BTreeMap::new(),
            fill_geometry: None,
            stroke_geometry: None,
            component: ComponentLink::default(),
            prototype: PrototypeProps::default(),
            variables: NodeVariables::default(),
            export_settings: Vec::new(),
            dev: DevInfo::default(),
            slide: None,
            brush: None,
            ai_versions: Vec::new(),
            motion: NodeMotion::default(),
        }
    }

    fn placed(id: NodeId, kind: NodeKind, r: Rect) -> Self {
        let mut n = Self::new(id, kind);
        n.transform = Affine::translate(r.x, r.y);
        n.size = Vec2::new(r.w, r.h);
        n
    }

    pub fn frame(id: NodeId, r: Rect) -> Self {
        let mut n = Self::placed(id, NodeKind::Frame(FrameProps::default()), r);
        n.fills = vec![Paint::solid(Color::WHITE)];
        n
    }
    pub fn rectangle(id: NodeId, r: Rect) -> Self {
        let mut n = Self::placed(id, NodeKind::Rectangle, r);
        n.fills = vec![Paint::solid(Color::rgb(0xD9, 0xD9, 0xD9))];
        n
    }
    pub fn ellipse(id: NodeId, r: Rect) -> Self {
        let mut n = Self::placed(id, NodeKind::Ellipse(ArcData::default()), r);
        n.fills = vec![Paint::solid(Color::rgb(0xD9, 0xD9, 0xD9))];
        n
    }
    pub fn text(id: NodeId, r: Rect, content: impl Into<String>, style: TextStyle) -> Self {
        Self::placed(
            id,
            NodeKind::Text(TextProps {
                content: content.into(),
                style,
                runs: Vec::new(),
                auto_resize: TextAutoResize::WidthAndHeight,
                max_lines: None,
                ..Default::default()
            }),
            r,
        )
    }

    /// Axis-aligned local bounds (0, 0, w, h).
    pub fn local_bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.size.x, self.size.y)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    Page {
        background: Color,
    },
    Frame(FrameProps),
    Group,
    Section,
    Rectangle,
    Ellipse(ArcData),
    Line,
    /// Regular polygon with `count` sides.
    Polygon {
        count: u32,
    },
    Star {
        count: u32,
        inner_ratio: f32,
    },
    /// Arbitrary vector geometry in local space.
    ///
    /// `paths` is the rendered geometry. When `network` is present it is the
    /// source of truth (vertices, segments, filled regions, per-vertex corner
    /// radius) and `paths` is derived from it with `VectorNetwork::to_paths`.
    Vector {
        paths: Vec<Path>,
        #[serde(default)]
        network: Option<VectorNetwork>,
    },
    Text(TextProps),
    BooleanOp(BooleanOp),
    /// Main component; frame-like.
    Component(FrameProps),
    ComponentSet(FrameProps),
    /// Instance of a component; children are the resolved (overridden) copy.
    Instance {
        frame: FrameProps,
        component: Option<NodeId>,
        component_key: Option<String>,
    },
    /// Board sticky note.
    Sticky(StickyProps),
    /// Board connector between two nodes (or free points); see [`ConnectorProps`].
    Connector(ConnectorProps),
    /// Board shape that carries centered text.
    ShapeWithText(ShapeWithTextProps),
    /// Board stamp: a small badge drawn from vector artwork (`kind` names the design).
    Stamp {
        kind: String,
    },
    /// Board section: a named, coloured container; see [`SectionProps`].
    BoardSection(SectionProps),
    /// Board table: a grid whose cells are its [`NodeKind::TableCell`] children (row-major).
    Table(TableProps),
    /// One cell of a [`NodeKind::Table`]; its box is derived from the table's grid.
    TableCell(TableCellProps),
    /// Board code block: monospace code with syntax highlighting.
    CodeBlock(CodeBlockProps),
    /// Anything the importer could not map, kept so nothing is silently lost.
    Unknown {
        type_name: String,
    },
}

impl NodeKind {
    pub fn is_container(&self) -> bool {
        matches!(
            self,
            NodeKind::Page { .. }
                | NodeKind::Frame(_)
                | NodeKind::Group
                | NodeKind::Section
                | NodeKind::BoardSection(_)
                | NodeKind::Table(_)
                | NodeKind::BooleanOp(_)
                | NodeKind::Component(_)
                | NodeKind::ComponentSet(_)
                | NodeKind::Instance { .. }
        )
    }

    /// Board items that are drawn by lowering to ordinary shapes and text.
    pub fn is_board_item(&self) -> bool {
        matches!(
            self,
            NodeKind::Sticky(_)
                | NodeKind::Connector(_)
                | NodeKind::ShapeWithText(_)
                | NodeKind::Stamp { .. }
                | NodeKind::BoardSection(_)
                | NodeKind::Table(_)
                | NodeKind::TableCell(_)
                | NodeKind::CodeBlock(_)
        )
    }

    /// The text of a board item that has its own text (sticky, shape with text), if any.
    pub fn board_text(&self) -> Option<&TextProps> {
        match self {
            NodeKind::Sticky(s) => Some(&s.text),
            NodeKind::ShapeWithText(s) => Some(&s.text),
            NodeKind::TableCell(c) => Some(&c.text),
            NodeKind::CodeBlock(c) => Some(&c.text),
            NodeKind::Connector(c) => c.label.as_ref(),
            _ => None,
        }
    }

    /// The editable text of a text layer or of a board item that carries text.
    pub fn any_text(&self) -> Option<&TextProps> {
        match self {
            NodeKind::Text(t) => Some(t),
            k => k.board_text(),
        }
    }

    pub fn any_text_mut(&mut self) -> Option<&mut TextProps> {
        match self {
            NodeKind::Text(t) => Some(t),
            k => k.board_text_mut(),
        }
    }

    pub fn board_text_mut(&mut self) -> Option<&mut TextProps> {
        match self {
            NodeKind::Sticky(s) => Some(&mut s.text),
            NodeKind::ShapeWithText(s) => Some(&mut s.text),
            NodeKind::TableCell(c) => Some(&mut c.text),
            NodeKind::CodeBlock(c) => Some(&mut c.text),
            NodeKind::Connector(c) => c.label.as_mut(),
            _ => None,
        }
    }

    pub fn frame_props(&self) -> Option<&FrameProps> {
        match self {
            NodeKind::Frame(f) | NodeKind::Component(f) | NodeKind::ComponentSet(f) => Some(f),
            NodeKind::Instance { frame, .. } => Some(frame),
            _ => None,
        }
    }

    pub fn frame_props_mut(&mut self) -> Option<&mut FrameProps> {
        match self {
            NodeKind::Frame(f) | NodeKind::Component(f) | NodeKind::ComponentSet(f) => Some(f),
            NodeKind::Instance { frame, .. } => Some(frame),
            _ => None,
        }
    }

    pub fn default_name(&self) -> &'static str {
        match self {
            NodeKind::Page { .. } => "Page",
            NodeKind::Frame(_) => "Frame",
            NodeKind::Group => "Group",
            NodeKind::Section => "Section",
            NodeKind::Rectangle => "Rectangle",
            NodeKind::Ellipse(_) => "Ellipse",
            NodeKind::Line => "Line",
            NodeKind::Polygon { .. } => "Polygon",
            NodeKind::Star { .. } => "Star",
            NodeKind::Vector { .. } => "Vector",
            NodeKind::Text(_) => "Text",
            NodeKind::BooleanOp(_) => "Union",
            NodeKind::Component(_) => "Component",
            NodeKind::ComponentSet(_) => "Component set",
            NodeKind::Instance { .. } => "Instance",
            NodeKind::Sticky(_) => "Sticky note",
            NodeKind::Connector(_) => "Connector",
            NodeKind::ShapeWithText(_) => "Shape",
            NodeKind::Stamp { .. } => "Stamp",
            NodeKind::BoardSection(_) => "Section",
            NodeKind::Table(_) => "Table",
            NodeKind::TableCell(_) => "Cell",
            NodeKind::CodeBlock(_) => "Code block",
            NodeKind::Unknown { .. } => "Layer",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BooleanOp {
    Union,
    Subtract,
    Intersect,
    Exclude,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArcData {
    pub start: f32,
    pub end: f32,
    pub inner_radius: f32,
}

impl Default for ArcData {
    fn default() -> Self {
        Self { start: 0.0, end: std::f32::consts::TAU, inner_radius: 0.0 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextProps {
    pub content: String,
    pub style: TextStyle,
    /// Style overrides over byte ranges of `content`; empty means uniform.
    pub runs: Vec<TextRun>,
    pub auto_resize: TextAutoResize,
    /// Maximum visible lines when `auto_resize` is `Truncate`; overflow ends in "…".
    /// `None` truncates at the box height.
    #[serde(default)]
    pub max_lines: Option<u32>,
    /// Paragraph settings (indent, list spacing, hanging punctuation / lists, leading trim).
    #[serde(default)]
    pub paragraph: ParagraphProps,
    /// Bulleted and numbered lists as byte ranges covering whole paragraphs.
    #[serde(default)]
    pub lists: Vec<ListRange>,
    /// Paragraph writing direction.
    #[serde(default)]
    pub direction: TextDirection,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FrameProps {
    pub clips_content: bool,
    pub auto_layout: Option<AutoLayout>,
    pub grids: Vec<LayoutGrid>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutDirection {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutAlign {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    Baseline,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sizing {
    #[default]
    Fixed,
    Hug,
    Fill,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AutoLayout {
    pub direction: LayoutDirection,
    pub wrap: bool,
    pub item_spacing: f32,
    pub counter_spacing: f32,
    /// top, right, bottom, left
    pub padding: [f32; 4],
    pub primary_align: LayoutAlign,
    pub counter_align: LayoutAlign,
    pub primary_sizing: Sizing,
    pub counter_sizing: Sizing,
    pub stroke_included: bool,
    pub reverse_z: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutChild {
    pub absolute: bool,
    pub horizontal: Sizing,
    pub vertical: Sizing,
    pub grow: f32,
    pub min_w: Option<f32>,
    pub max_w: Option<f32>,
    pub min_h: Option<f32>,
    pub max_h: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Constraint {
    #[default]
    Min,
    Max,
    Center,
    Stretch,
    Scale,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraints {
    pub horizontal: Constraint,
    pub vertical: Constraint,
}
