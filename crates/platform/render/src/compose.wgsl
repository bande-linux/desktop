struct Quad {
    color: vec4<f32>,
    target_size: vec2<f32>,
    origin: vec2<f32>,
    size: vec2<f32>,
    source_origin: vec2<f32>,
    source_size: vec2<f32>,
    alpha: f32,
    padding: f32,
}

var<immediate> quad: Quad;

@group(0) @binding(0) var surface: texture_2d<f32>;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let pixel = quad.origin + corner * quad.size;
    return vec4<f32>(2.0 * pixel / quad.target_size - 1.0, 0.0, 1.0);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let local = (position.xy - quad.origin) / quad.size;
    let source = quad.source_origin + local * quad.source_size;
    let last = vec2<i32>(textureDimensions(surface)) - 1;
    let texel = clamp(vec2<i32>(floor(source)), vec2<i32>(0), last);
    return textureLoad(surface, texel, 0) * quad.alpha;
}

@fragment
fn solid() -> @location(0) vec4<f32> {
    return quad.color;
}
