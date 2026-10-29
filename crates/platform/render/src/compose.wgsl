struct Quad {
    color: vec4<f32>,
    target_size: vec2<f32>,
    origin: vec2<f32>,
    size: vec2<f32>,
    source_origin: vec2<f32>,
    source_size: vec2<f32>,
    alpha: f32,
    blur: f32,
    clip: vec4<f32>,
    caster: vec4<f32>,
    clip_radius: f32,
    caster_radius: f32,
    padding: vec2<f32>,
}

var<immediate> quad: Quad;

@group(0) @binding(0) var surface: texture_2d<f32>;
@group(0) @binding(1) var filtered: sampler;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let pixel = quad.origin + corner * quad.size;
    return vec4<f32>(2.0 * pixel / quad.target_size - 1.0, 0.0, 1.0);
}

fn rounded_distance(point: vec2<f32>, rect: vec4<f32>, radius: f32) -> f32 {
    let half = rect.zw * 0.5;
    let corner = min(radius, min(half.x, half.y));
    let inset = abs(point - rect.xy - half) - half + corner;
    return length(max(inset, vec2<f32>(0.0))) + min(max(inset.x, inset.y), 0.0) - corner;
}

fn coverage(point: vec2<f32>, rect: vec4<f32>, radius: f32) -> f32 {
    if rect.z <= 0.0 || rect.w <= 0.0 {
        return 1.0;
    }
    return clamp(0.5 - rounded_distance(point, rect, radius), 0.0, 1.0);
}

fn erf(x: vec2<f32>) -> vec2<f32> {
    let magnitude = abs(x);
    let polynomial = 1.0 + (0.278393 + (0.230389 + 0.000972 * magnitude + 0.078108 * magnitude * magnitude) * magnitude) * magnitude;
    let squared = polynomial * polynomial;
    return sign(x) - sign(x) / (squared * squared);
}

fn gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (2.5066283 * sigma);
}

fn shadow_row(x: f32, y: f32, sigma: f32, corner: f32, half: vec2<f32>) -> f32 {
    let delta = min(half.y - corner - abs(y), 0.0);
    let curved = half.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
    let integral = 0.5 + 0.5 * erf((x + vec2<f32>(-curved, curved)) * (0.70710678 / sigma));
    return integral.y - integral.x;
}

fn box_shadow(point: vec2<f32>, rect: vec4<f32>, radius: f32, sigma: f32) -> f32 {
    let half = rect.zw * 0.5;
    let corner = min(radius, min(half.x, half.y));
    let local = point - rect.xy - half;
    let low = local.y - half.y;
    let high = local.y + half.y;
    let start = clamp(-3.0 * sigma, low, high);
    let end = clamp(3.0 * sigma, low, high);
    let step = (end - start) / 4.0;
    var y = start + step * 0.5;
    var value = 0.0;
    for (var sample = 0; sample < 4; sample++) {
        value += shadow_row(local.x, local.y - y, sigma, corner, half) * gaussian(y, sigma) * step;
        y += step;
    }
    return value;
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let local = (position.xy - quad.origin) / quad.size;
    let source = quad.source_origin + local * quad.source_size;
    let dimensions = vec2<f32>(textureDimensions(surface));
    var texel: vec4<f32>;
    if all(abs(quad.source_size - quad.size) < vec2<f32>(0.001)) {
        let last = vec2<i32>(dimensions) - 1;
        texel = textureLoad(surface, clamp(vec2<i32>(floor(source)), vec2<i32>(0), last), 0);
    } else {
        texel = textureSampleLevel(surface, filtered, source / dimensions, 0.0);
    }
    return texel * quad.alpha * coverage(position.xy, quad.clip, quad.clip_radius);
}

@fragment
fn solid(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return quad.color * coverage(position.xy, quad.clip, quad.clip_radius);
}

@fragment
fn shadow(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let falloff = box_shadow(position.xy, quad.caster, quad.caster_radius, quad.blur);
    let uncovered = 1.0 - coverage(position.xy, quad.clip, quad.clip_radius);
    return quad.color * (falloff * uncovered * quad.alpha);
}

fn sample_at(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(surface, filtered, uv, 0.0);
}

@fragment
fn blur_down(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = position.xy / quad.size;
    let half_texel = quad.blur * 0.5 / quad.source_size;
    let diagonal = vec2<f32>(half_texel.x, -half_texel.y);
    return (sample_at(uv) * 4.0 + sample_at(uv - half_texel) + sample_at(uv + half_texel)
        + sample_at(uv + diagonal) + sample_at(uv - diagonal)) / 8.0;
}

@fragment
fn blur_up(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = position.xy / quad.size;
    let h = quad.blur * 0.5 / quad.size;
    let edges = sample_at(uv + vec2<f32>(-2.0 * h.x, 0.0)) + sample_at(uv + vec2<f32>(2.0 * h.x, 0.0))
        + sample_at(uv + vec2<f32>(0.0, -2.0 * h.y)) + sample_at(uv + vec2<f32>(0.0, 2.0 * h.y));
    let corners = sample_at(uv + vec2<f32>(-h.x, h.y)) + sample_at(uv + vec2<f32>(h.x, h.y))
        + sample_at(uv + vec2<f32>(h.x, -h.y)) + sample_at(uv + vec2<f32>(-h.x, -h.y));
    return (edges + corners * 2.0) / 12.0;
}
