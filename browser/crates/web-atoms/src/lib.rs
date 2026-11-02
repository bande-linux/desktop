use std::fmt;
use std::hash::{BuildHasher, RandomState};

use hashbrown::HashTable;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Atom(u32);

macro_rules! static_atoms {
    ($($constant:ident = $name:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u32)]
        enum Static { $($constant,)* }

        impl Atom {
            $(pub const $constant: Atom = Atom(Static::$constant as u32);)*

            pub fn from_static(name: &str) -> Option<Atom> {
                match name {
                    $($name => Some(Atom::$constant),)*
                    _ => None,
                }
            }
        }

        const STATIC_NAMES: &[&str] = &[$($name,)*];
    };
}

static_atoms! {
    A = "a",
    ABBR = "abbr",
    ACRONYM = "acronym",
    ADDRESS = "address",
    ANNOTATION_XML = "annotation-xml",
    APPLET = "applet",
    AREA = "area",
    ARTICLE = "article",
    ASIDE = "aside",
    AUDIO = "audio",
    B = "b",
    BASE = "base",
    BASEFONT = "basefont",
    BDI = "bdi",
    BDO = "bdo",
    BGSOUND = "bgsound",
    BIG = "big",
    BLINK = "blink",
    BLOCKQUOTE = "blockquote",
    BODY = "body",
    BR = "br",
    BUTTON = "button",
    CANVAS = "canvas",
    CAPTION = "caption",
    CENTER = "center",
    CITE = "cite",
    CODE = "code",
    COL = "col",
    COLGROUP = "colgroup",
    DATA = "data",
    DATALIST = "datalist",
    DD = "dd",
    DEL = "del",
    DESC = "desc",
    DETAILS = "details",
    DFN = "dfn",
    DIALOG = "dialog",
    DIR = "dir",
    DIV = "div",
    DL = "dl",
    DT = "dt",
    EM = "em",
    EMBED = "embed",
    FIELDSET = "fieldset",
    FIGCAPTION = "figcaption",
    FIGURE = "figure",
    FONT = "font",
    FOOTER = "footer",
    FOREIGN_OBJECT = "foreignObject",
    FORM = "form",
    FRAME = "frame",
    FRAMESET = "frameset",
    H1 = "h1",
    H2 = "h2",
    H3 = "h3",
    H4 = "h4",
    H5 = "h5",
    H6 = "h6",
    HEAD = "head",
    HEADER = "header",
    HGROUP = "hgroup",
    HR = "hr",
    HTML = "html",
    I = "i",
    IFRAME = "iframe",
    IMAGE = "image",
    IMG = "img",
    INPUT = "input",
    INS = "ins",
    ISINDEX = "isindex",
    KBD = "kbd",
    KEYGEN = "keygen",
    LABEL = "label",
    LEGEND = "legend",
    LI = "li",
    LINK = "link",
    LISTING = "listing",
    MAIN = "main",
    MALIGNMARK = "malignmark",
    MAP = "map",
    MARK = "mark",
    MARQUEE = "marquee",
    MATH = "math",
    MENU = "menu",
    MENUITEM = "menuitem",
    META = "meta",
    METER = "meter",
    MGLYPH = "mglyph",
    MI = "mi",
    MN = "mn",
    MO = "mo",
    MS = "ms",
    MTEXT = "mtext",
    NAV = "nav",
    NOBR = "nobr",
    NOEMBED = "noembed",
    NOFRAMES = "noframes",
    NOSCRIPT = "noscript",
    OBJECT = "object",
    OL = "ol",
    OPTGROUP = "optgroup",
    OPTION = "option",
    OUTPUT = "output",
    P = "p",
    PARAM = "param",
    PICTURE = "picture",
    PLAINTEXT = "plaintext",
    PRE = "pre",
    PROGRESS = "progress",
    Q = "q",
    RB = "rb",
    RP = "rp",
    RT = "rt",
    RTC = "rtc",
    RUBY = "ruby",
    S = "s",
    SAMP = "samp",
    SCRIPT = "script",
    SEARCH = "search",
    SECTION = "section",
    SELECT = "select",
    SLOT = "slot",
    SMALL = "small",
    SOURCE = "source",
    SPAN = "span",
    STRIKE = "strike",
    STRONG = "strong",
    STYLE = "style",
    SUB = "sub",
    SUMMARY = "summary",
    SUP = "sup",
    SVG = "svg",
    TABLE = "table",
    TBODY = "tbody",
    TD = "td",
    TEMPLATE = "template",
    TEXTAREA = "textarea",
    TFOOT = "tfoot",
    TH = "th",
    THEAD = "thead",
    TIME = "time",
    TITLE = "title",
    TR = "tr",
    TRACK = "track",
    TT = "tt",
    U = "u",
    UL = "ul",
    VAR = "var",
    VIDEO = "video",
    WBR = "wbr",
    XMP = "xmp",
    ACTION = "action",
    ALIGN = "align",
    ALT = "alt",
    CHARSET = "charset",
    CLASS = "class",
    COLOR = "color",
    COLSPAN = "colspan",
    CONTENT = "content",
    DEFINITIONURL = "definitionurl",
    ENCODING = "encoding",
    FACE = "face",
    HEIGHT = "height",
    HIDDEN = "hidden",
    HREF = "href",
    HTTP_EQUIV = "http-equiv",
    ID = "id",
    LANG = "lang",
    METHOD = "method",
    NAME = "name",
    PROMPT = "prompt",
    REL = "rel",
    REVERSED = "reversed",
    ROWSPAN = "rowspan",
    SIZE = "size",
    SRC = "src",
    START = "start",
    TARGET = "target",
    TYPE = "type",
    VALUE = "value",
    WIDTH = "width",
    XMLNS = "xmlns",
    VALIGN = "valign",
    NOWRAP = "nowrap",
    BGCOLOR = "bgcolor",
    MEDIA = "media",
    SELECTED = "selected",
    REQUIRED = "required",
    READONLY = "readonly",
    OPEN = "open",
    DISABLED = "disabled",
    CHECKED = "checked",
    ACTUATE = "actuate",
    ALT_GLYPH = "altGlyph",
    ALT_GLYPH_DEF = "altGlyphDef",
    ALT_GLYPH_ITEM = "altGlyphItem",
    ALTGLYPH = "altglyph",
    ALTGLYPHDEF = "altglyphdef",
    ALTGLYPHITEM = "altglyphitem",
    ANIMATE_COLOR = "animateColor",
    ANIMATE_MOTION = "animateMotion",
    ANIMATE_TRANSFORM = "animateTransform",
    ANIMATECOLOR = "animatecolor",
    ANIMATEMOTION = "animatemotion",
    ANIMATETRANSFORM = "animatetransform",
    ARCROLE = "arcrole",
    ATTRIBUTE_NAME = "attributeName",
    ATTRIBUTE_TYPE = "attributeType",
    ATTRIBUTENAME = "attributename",
    ATTRIBUTETYPE = "attributetype",
    BASE_FREQUENCY = "baseFrequency",
    BASE_PROFILE = "baseProfile",
    BASEFREQUENCY = "basefrequency",
    BASEPROFILE = "baseprofile",
    CALC_MODE = "calcMode",
    CALCMODE = "calcmode",
    CLIP_PATH = "clipPath",
    CLIP_PATH_UNITS = "clipPathUnits",
    CLIPPATH = "clippath",
    CLIPPATHUNITS = "clippathunits",
    DEFINITION_URL = "definitionURL",
    DIFFUSE_CONSTANT = "diffuseConstant",
    DIFFUSECONSTANT = "diffuseconstant",
    EDGE_MODE = "edgeMode",
    EDGEMODE = "edgemode",
    FE_BLEND = "feBlend",
    FE_COLOR_MATRIX = "feColorMatrix",
    FE_COMPONENT_TRANSFER = "feComponentTransfer",
    FE_COMPOSITE = "feComposite",
    FE_CONVOLVE_MATRIX = "feConvolveMatrix",
    FE_DIFFUSE_LIGHTING = "feDiffuseLighting",
    FE_DISPLACEMENT_MAP = "feDisplacementMap",
    FE_DISTANT_LIGHT = "feDistantLight",
    FE_DROP_SHADOW = "feDropShadow",
    FE_FLOOD = "feFlood",
    FE_FUNC_A = "feFuncA",
    FE_FUNC_B = "feFuncB",
    FE_FUNC_G = "feFuncG",
    FE_FUNC_R = "feFuncR",
    FE_GAUSSIAN_BLUR = "feGaussianBlur",
    FE_IMAGE = "feImage",
    FE_MERGE = "feMerge",
    FE_MERGE_NODE = "feMergeNode",
    FE_MORPHOLOGY = "feMorphology",
    FE_OFFSET = "feOffset",
    FE_POINT_LIGHT = "fePointLight",
    FE_SPECULAR_LIGHTING = "feSpecularLighting",
    FE_SPOT_LIGHT = "feSpotLight",
    FE_TILE = "feTile",
    FE_TURBULENCE = "feTurbulence",
    FEBLEND = "feblend",
    FECOLORMATRIX = "fecolormatrix",
    FECOMPONENTTRANSFER = "fecomponenttransfer",
    FECOMPOSITE = "fecomposite",
    FECONVOLVEMATRIX = "feconvolvematrix",
    FEDIFFUSELIGHTING = "fediffuselighting",
    FEDISPLACEMENTMAP = "fedisplacementmap",
    FEDISTANTLIGHT = "fedistantlight",
    FEDROPSHADOW = "fedropshadow",
    FEFLOOD = "feflood",
    FEFUNCA = "fefunca",
    FEFUNCB = "fefuncb",
    FEFUNCG = "fefuncg",
    FEFUNCR = "fefuncr",
    FEGAUSSIANBLUR = "fegaussianblur",
    FEIMAGE = "feimage",
    FEMERGE = "femerge",
    FEMERGENODE = "femergenode",
    FEMORPHOLOGY = "femorphology",
    FEOFFSET = "feoffset",
    FEPOINTLIGHT = "fepointlight",
    FESPECULARLIGHTING = "fespecularlighting",
    FESPOTLIGHT = "fespotlight",
    FETILE = "fetile",
    FETURBULENCE = "feturbulence",
    FILTER_UNITS = "filterUnits",
    FILTERUNITS = "filterunits",
    FOREIGNOBJECT = "foreignobject",
    GLYPH_REF = "glyphRef",
    GLYPHREF = "glyphref",
    GRADIENT_TRANSFORM = "gradientTransform",
    GRADIENT_UNITS = "gradientUnits",
    GRADIENTTRANSFORM = "gradienttransform",
    GRADIENTUNITS = "gradientunits",
    KERNEL_MATRIX = "kernelMatrix",
    KERNEL_UNIT_LENGTH = "kernelUnitLength",
    KERNELMATRIX = "kernelmatrix",
    KERNELUNITLENGTH = "kernelunitlength",
    KEY_POINTS = "keyPoints",
    KEY_SPLINES = "keySplines",
    KEY_TIMES = "keyTimes",
    KEYPOINTS = "keypoints",
    KEYSPLINES = "keysplines",
    KEYTIMES = "keytimes",
    LENGTH_ADJUST = "lengthAdjust",
    LENGTHADJUST = "lengthadjust",
    LIMITING_CONE_ANGLE = "limitingConeAngle",
    LIMITINGCONEANGLE = "limitingconeangle",
    LINEAR_GRADIENT = "linearGradient",
    LINEARGRADIENT = "lineargradient",
    MARKER_HEIGHT = "markerHeight",
    MARKER_UNITS = "markerUnits",
    MARKER_WIDTH = "markerWidth",
    MARKERHEIGHT = "markerheight",
    MARKERUNITS = "markerunits",
    MARKERWIDTH = "markerwidth",
    MASK_CONTENT_UNITS = "maskContentUnits",
    MASK_UNITS = "maskUnits",
    MASKCONTENTUNITS = "maskcontentunits",
    MASKUNITS = "maskunits",
    NUM_OCTAVES = "numOctaves",
    NUMOCTAVES = "numoctaves",
    PATH_LENGTH = "pathLength",
    PATHLENGTH = "pathlength",
    PATTERN_CONTENT_UNITS = "patternContentUnits",
    PATTERN_TRANSFORM = "patternTransform",
    PATTERN_UNITS = "patternUnits",
    PATTERNCONTENTUNITS = "patterncontentunits",
    PATTERNTRANSFORM = "patterntransform",
    PATTERNUNITS = "patternunits",
    POINTS_AT_X = "pointsAtX",
    POINTS_AT_Y = "pointsAtY",
    POINTS_AT_Z = "pointsAtZ",
    POINTSATX = "pointsatx",
    POINTSATY = "pointsaty",
    POINTSATZ = "pointsatz",
    PRESERVE_ALPHA = "preserveAlpha",
    PRESERVE_ASPECT_RATIO = "preserveAspectRatio",
    PRESERVEALPHA = "preservealpha",
    PRESERVEASPECTRATIO = "preserveaspectratio",
    PRIMITIVE_UNITS = "primitiveUnits",
    PRIMITIVEUNITS = "primitiveunits",
    RADIAL_GRADIENT = "radialGradient",
    RADIALGRADIENT = "radialgradient",
    REF_X = "refX",
    REF_Y = "refY",
    REFX = "refx",
    REFY = "refy",
    REPEAT_COUNT = "repeatCount",
    REPEAT_DUR = "repeatDur",
    REPEATCOUNT = "repeatcount",
    REPEATDUR = "repeatdur",
    REQUIRED_EXTENSIONS = "requiredExtensions",
    REQUIRED_FEATURES = "requiredFeatures",
    REQUIREDEXTENSIONS = "requiredextensions",
    REQUIREDFEATURES = "requiredfeatures",
    ROLE = "role",
    SHOW = "show",
    SPACE = "space",
    SPECULAR_CONSTANT = "specularConstant",
    SPECULAR_EXPONENT = "specularExponent",
    SPECULARCONSTANT = "specularconstant",
    SPECULAREXPONENT = "specularexponent",
    SPREAD_METHOD = "spreadMethod",
    SPREADMETHOD = "spreadmethod",
    START_OFFSET = "startOffset",
    STARTOFFSET = "startoffset",
    STD_DEVIATION = "stdDeviation",
    STDDEVIATION = "stddeviation",
    STITCH_TILES = "stitchTiles",
    STITCHTILES = "stitchtiles",
    SURFACE_SCALE = "surfaceScale",
    SURFACESCALE = "surfacescale",
    SYSTEM_LANGUAGE = "systemLanguage",
    SYSTEMLANGUAGE = "systemlanguage",
    TABLE_VALUES = "tableValues",
    TABLEVALUES = "tablevalues",
    TARGET_X = "targetX",
    TARGET_Y = "targetY",
    TARGETX = "targetx",
    TARGETY = "targety",
    TEXT_LENGTH = "textLength",
    TEXT_PATH = "textPath",
    TEXTLENGTH = "textlength",
    TEXTPATH = "textpath",
    VIEW_BOX = "viewBox",
    VIEW_TARGET = "viewTarget",
    VIEWBOX = "viewbox",
    VIEWTARGET = "viewtarget",
    X_CHANNEL_SELECTOR = "xChannelSelector",
    XCHANNELSELECTOR = "xchannelselector",
    XLINK = "xlink",
    XLINK_ACTUATE = "xlink:actuate",
    XLINK_ARCROLE = "xlink:arcrole",
    XLINK_HREF = "xlink:href",
    XLINK_ROLE = "xlink:role",
    XLINK_SHOW = "xlink:show",
    XLINK_TITLE = "xlink:title",
    XLINK_TYPE = "xlink:type",
    XML_LANG = "xml:lang",
    XML_SPACE = "xml:space",
    XMLNS_XLINK = "xmlns:xlink",
    Y_CHANNEL_SELECTOR = "yChannelSelector",
    YCHANNELSELECTOR = "ychannelselector",
    ZOOM_AND_PAN = "zoomAndPan",
    ZOOMANDPAN = "zoomandpan",
}

impl Atom {
    pub fn is_static(self) -> bool {
        (self.0 as usize) < STATIC_NAMES.len()
    }

    pub fn static_name(self) -> Option<&'static str> {
        STATIC_NAMES.get(self.0 as usize).copied()
    }

    pub fn index(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for Atom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.static_name() {
            Some(name) => write!(f, "Atom({name})"),
            None => write!(f, "Atom(#{})", self.0),
        }
    }
}

#[derive(Default)]
pub struct AtomTable {
    text: String,
    spans: Vec<(u32, u32)>,
    lookup: HashTable<u32>,
    hasher: RandomState,
}

impl AtomTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, name: &str) -> Atom {
        if let Some(atom) = self.get(name) {
            return atom;
        }
        let start = self.text.len() as u32;
        self.text.push_str(name);
        self.spans.push((start, name.len() as u32));
        let slot = (self.spans.len() - 1) as u32;
        let hasher = &self.hasher;
        let (text, spans) = (&self.text, &self.spans);
        self.lookup
            .insert_unique(hasher.hash_one(name), slot, |&slot| {
                hasher.hash_one(dynamic_name(text, spans, slot))
            });
        Atom(STATIC_NAMES.len() as u32 + slot)
    }

    pub fn get(&self, name: &str) -> Option<Atom> {
        if let Some(atom) = Atom::from_static(name) {
            return Some(atom);
        }
        self.lookup
            .find(self.hasher.hash_one(name), |&slot| {
                dynamic_name(&self.text, &self.spans, slot) == name
            })
            .map(|&slot| Atom(STATIC_NAMES.len() as u32 + slot))
    }

    pub fn heap_bytes(&self) -> usize {
        self.text.capacity()
            + self.spans.capacity() * size_of::<(u32, u32)>()
            + self.lookup.capacity() * (size_of::<u32>() + 1)
    }

    pub fn name(&self, atom: Atom) -> &str {
        match atom.static_name() {
            Some(name) => name,
            None => dynamic_name(&self.text, &self.spans, atom.0 - STATIC_NAMES.len() as u32),
        }
    }
}

fn dynamic_name<'a>(text: &'a str, spans: &[(u32, u32)], slot: u32) -> &'a str {
    spans
        .get(slot as usize)
        .and_then(|&(start, len)| text.get(start as usize..(start + len) as usize))
        .unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(u8)]
pub enum Namespace {
    #[default]
    Html,
    Svg,
    MathMl,
    None,
    XLink,
    Xml,
    Xmlns,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct QualName {
    pub ns: Namespace,
    pub local: Atom,
}

impl QualName {
    pub const fn html(local: Atom) -> Self {
        QualName {
            ns: Namespace::Html,
            local,
        }
    }

    pub const fn attribute(local: Atom) -> Self {
        QualName {
            ns: Namespace::None,
            local,
        }
    }
}
