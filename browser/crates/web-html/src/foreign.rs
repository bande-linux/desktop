use web_atoms::{Atom, Namespace, QualName};

use crate::tokenizer::Tag;

pub(crate) fn svg_tag_name(name: Atom) -> Atom {
    match name {
        Atom::ALTGLYPH => Atom::ALT_GLYPH,
        Atom::ALTGLYPHDEF => Atom::ALT_GLYPH_DEF,
        Atom::ALTGLYPHITEM => Atom::ALT_GLYPH_ITEM,
        Atom::ANIMATECOLOR => Atom::ANIMATE_COLOR,
        Atom::ANIMATEMOTION => Atom::ANIMATE_MOTION,
        Atom::ANIMATETRANSFORM => Atom::ANIMATE_TRANSFORM,
        Atom::CLIPPATH => Atom::CLIP_PATH,
        Atom::FEBLEND => Atom::FE_BLEND,
        Atom::FECOLORMATRIX => Atom::FE_COLOR_MATRIX,
        Atom::FECOMPONENTTRANSFER => Atom::FE_COMPONENT_TRANSFER,
        Atom::FECOMPOSITE => Atom::FE_COMPOSITE,
        Atom::FECONVOLVEMATRIX => Atom::FE_CONVOLVE_MATRIX,
        Atom::FEDIFFUSELIGHTING => Atom::FE_DIFFUSE_LIGHTING,
        Atom::FEDISPLACEMENTMAP => Atom::FE_DISPLACEMENT_MAP,
        Atom::FEDISTANTLIGHT => Atom::FE_DISTANT_LIGHT,
        Atom::FEDROPSHADOW => Atom::FE_DROP_SHADOW,
        Atom::FEFLOOD => Atom::FE_FLOOD,
        Atom::FEFUNCA => Atom::FE_FUNC_A,
        Atom::FEFUNCB => Atom::FE_FUNC_B,
        Atom::FEFUNCG => Atom::FE_FUNC_G,
        Atom::FEFUNCR => Atom::FE_FUNC_R,
        Atom::FEGAUSSIANBLUR => Atom::FE_GAUSSIAN_BLUR,
        Atom::FEIMAGE => Atom::FE_IMAGE,
        Atom::FEMERGE => Atom::FE_MERGE,
        Atom::FEMERGENODE => Atom::FE_MERGE_NODE,
        Atom::FEMORPHOLOGY => Atom::FE_MORPHOLOGY,
        Atom::FEOFFSET => Atom::FE_OFFSET,
        Atom::FEPOINTLIGHT => Atom::FE_POINT_LIGHT,
        Atom::FESPECULARLIGHTING => Atom::FE_SPECULAR_LIGHTING,
        Atom::FESPOTLIGHT => Atom::FE_SPOT_LIGHT,
        Atom::FETILE => Atom::FE_TILE,
        Atom::FETURBULENCE => Atom::FE_TURBULENCE,
        Atom::FOREIGNOBJECT => Atom::FOREIGN_OBJECT,
        Atom::GLYPHREF => Atom::GLYPH_REF,
        Atom::LINEARGRADIENT => Atom::LINEAR_GRADIENT,
        Atom::RADIALGRADIENT => Atom::RADIAL_GRADIENT,
        Atom::TEXTPATH => Atom::TEXT_PATH,
        other => other,
    }
}

fn svg_attribute_name(name: Atom) -> Atom {
    match name {
        Atom::ATTRIBUTENAME => Atom::ATTRIBUTE_NAME,
        Atom::ATTRIBUTETYPE => Atom::ATTRIBUTE_TYPE,
        Atom::BASEFREQUENCY => Atom::BASE_FREQUENCY,
        Atom::BASEPROFILE => Atom::BASE_PROFILE,
        Atom::CALCMODE => Atom::CALC_MODE,
        Atom::CLIPPATHUNITS => Atom::CLIP_PATH_UNITS,
        Atom::DIFFUSECONSTANT => Atom::DIFFUSE_CONSTANT,
        Atom::EDGEMODE => Atom::EDGE_MODE,
        Atom::FILTERUNITS => Atom::FILTER_UNITS,
        Atom::GLYPHREF => Atom::GLYPH_REF,
        Atom::GRADIENTTRANSFORM => Atom::GRADIENT_TRANSFORM,
        Atom::GRADIENTUNITS => Atom::GRADIENT_UNITS,
        Atom::KERNELMATRIX => Atom::KERNEL_MATRIX,
        Atom::KERNELUNITLENGTH => Atom::KERNEL_UNIT_LENGTH,
        Atom::KEYPOINTS => Atom::KEY_POINTS,
        Atom::KEYSPLINES => Atom::KEY_SPLINES,
        Atom::KEYTIMES => Atom::KEY_TIMES,
        Atom::LENGTHADJUST => Atom::LENGTH_ADJUST,
        Atom::LIMITINGCONEANGLE => Atom::LIMITING_CONE_ANGLE,
        Atom::MARKERHEIGHT => Atom::MARKER_HEIGHT,
        Atom::MARKERUNITS => Atom::MARKER_UNITS,
        Atom::MARKERWIDTH => Atom::MARKER_WIDTH,
        Atom::MASKCONTENTUNITS => Atom::MASK_CONTENT_UNITS,
        Atom::MASKUNITS => Atom::MASK_UNITS,
        Atom::NUMOCTAVES => Atom::NUM_OCTAVES,
        Atom::PATHLENGTH => Atom::PATH_LENGTH,
        Atom::PATTERNCONTENTUNITS => Atom::PATTERN_CONTENT_UNITS,
        Atom::PATTERNTRANSFORM => Atom::PATTERN_TRANSFORM,
        Atom::PATTERNUNITS => Atom::PATTERN_UNITS,
        Atom::POINTSATX => Atom::POINTS_AT_X,
        Atom::POINTSATY => Atom::POINTS_AT_Y,
        Atom::POINTSATZ => Atom::POINTS_AT_Z,
        Atom::PRESERVEALPHA => Atom::PRESERVE_ALPHA,
        Atom::PRESERVEASPECTRATIO => Atom::PRESERVE_ASPECT_RATIO,
        Atom::PRIMITIVEUNITS => Atom::PRIMITIVE_UNITS,
        Atom::REFX => Atom::REF_X,
        Atom::REFY => Atom::REF_Y,
        Atom::REPEATCOUNT => Atom::REPEAT_COUNT,
        Atom::REPEATDUR => Atom::REPEAT_DUR,
        Atom::REQUIREDEXTENSIONS => Atom::REQUIRED_EXTENSIONS,
        Atom::REQUIREDFEATURES => Atom::REQUIRED_FEATURES,
        Atom::SPECULARCONSTANT => Atom::SPECULAR_CONSTANT,
        Atom::SPECULAREXPONENT => Atom::SPECULAR_EXPONENT,
        Atom::SPREADMETHOD => Atom::SPREAD_METHOD,
        Atom::STARTOFFSET => Atom::START_OFFSET,
        Atom::STDDEVIATION => Atom::STD_DEVIATION,
        Atom::STITCHTILES => Atom::STITCH_TILES,
        Atom::SURFACESCALE => Atom::SURFACE_SCALE,
        Atom::SYSTEMLANGUAGE => Atom::SYSTEM_LANGUAGE,
        Atom::TABLEVALUES => Atom::TABLE_VALUES,
        Atom::TARGETX => Atom::TARGET_X,
        Atom::TARGETY => Atom::TARGET_Y,
        Atom::TEXTLENGTH => Atom::TEXT_LENGTH,
        Atom::VIEWBOX => Atom::VIEW_BOX,
        Atom::VIEWTARGET => Atom::VIEW_TARGET,
        Atom::XCHANNELSELECTOR => Atom::X_CHANNEL_SELECTOR,
        Atom::YCHANNELSELECTOR => Atom::Y_CHANNEL_SELECTOR,
        Atom::ZOOMANDPAN => Atom::ZOOM_AND_PAN,
        other => other,
    }
}

fn foreign_attribute_name(name: Atom) -> Option<QualName> {
    let (ns, local) = match name {
        Atom::XLINK_ACTUATE => (Namespace::XLink, Atom::ACTUATE),
        Atom::XLINK_ARCROLE => (Namespace::XLink, Atom::ARCROLE),
        Atom::XLINK_HREF => (Namespace::XLink, Atom::HREF),
        Atom::XLINK_ROLE => (Namespace::XLink, Atom::ROLE),
        Atom::XLINK_SHOW => (Namespace::XLink, Atom::SHOW),
        Atom::XLINK_TITLE => (Namespace::XLink, Atom::TITLE),
        Atom::XLINK_TYPE => (Namespace::XLink, Atom::TYPE),
        Atom::XML_LANG => (Namespace::Xml, Atom::LANG),
        Atom::XML_SPACE => (Namespace::Xml, Atom::SPACE),
        Atom::XMLNS => (Namespace::Xmlns, Atom::XMLNS),
        Atom::XMLNS_XLINK => (Namespace::Xmlns, Atom::XLINK),
        _ => return None,
    };
    Some(QualName { ns, local })
}

pub(crate) fn adjust_svg_attributes(tag: &mut Tag) {
    for attr in &mut tag.attrs {
        attr.name.local = svg_attribute_name(attr.name.local);
    }
}

pub(crate) fn adjust_mathml_attributes(tag: &mut Tag) {
    for attr in &mut tag.attrs {
        if attr.name.local == Atom::DEFINITIONURL {
            attr.name.local = Atom::DEFINITION_URL;
        }
    }
}

pub(crate) fn adjust_foreign_attributes(tag: &mut Tag) {
    for attr in &mut tag.attrs {
        if let Some(name) = foreign_attribute_name(attr.name.local) {
            attr.name = name;
        }
    }
}
