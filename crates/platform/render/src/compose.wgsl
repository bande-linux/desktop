struct Quad {
    color: vec4<f32>,
    target_size: vec2<f32>,
    origin: vec2<f32>,
    size: vec2<f32>,
    source_origin: vec2<f32>,
    source_across: vec2<f32>,
    alpha: f32,
    blur: f32,
    clip: vec4<f32>,
    caster: vec4<f32>,
    clip_radius: f32,
    caster_radius: f32,
    source_down: vec2<f32>,
    dither: f32,
}

var<immediate> quad: Quad;

@group(0) @binding(0) var surface: texture_2d<f32>;
@group(0) @binding(1) var filtered: sampler;
@group(0) @binding(2) var chroma: texture_2d<f32>;

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

fn noise(position: vec4<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(floor(position.xy), vec2<f32>(0.06711056, 0.00583715)))) - 0.5;
}

fn dithered(color: vec4<f32>, position: vec4<f32>) -> vec4<f32> {
    if quad.dither == 0.0 {
        return color;
    }
    let visible = select(0.0, quad.dither, color.a > 0.0);
    return vec4<f32>(max(color.rgb + noise(position) * visible, vec3<f32>(0.0)), color.a);
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
    let source = quad.source_origin + local.x * quad.source_across + local.y * quad.source_down;
    let dimensions = vec2<f32>(textureDimensions(surface));
    let stretch = vec2<f32>(length(quad.source_across), length(quad.source_down));
    var texel: vec4<f32>;
    if all(abs(stretch - quad.size) < vec2<f32>(0.001)) {
        let last = vec2<i32>(dimensions) - 1;
        texel = textureLoad(surface, clamp(vec2<i32>(floor(source)), vec2<i32>(0), last), 0);
    } else {
        texel = textureSampleLevel(surface, filtered, source / dimensions, 0.0);
    }
    return dithered(texel * quad.alpha * coverage(position.xy, quad.clip, quad.clip_radius), position);
}

fn video_rgb(uv: vec2<f32>, red: f32, green: vec2<f32>, blue: f32) -> vec3<f32> {
    let luma = (textureSampleLevel(surface, filtered, uv, 0.0).r - 16.0 / 255.0) * 255.0 / 219.0;
    let difference = (textureSampleLevel(chroma, filtered, uv, 0.0).rg - vec2<f32>(128.0 / 255.0)) * 255.0 / 224.0;
    let rgb = vec3<f32>(
        luma + red * difference.y,
        luma - dot(green, difference),
        luma + blue * difference.x,
    );
    return clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn source_at(position: vec4<f32>) -> vec2<f32> {
    let local = (position.xy - quad.origin) / quad.size;
    let source = quad.source_origin + local.x * quad.source_across + local.y * quad.source_down;
    return source / vec2<f32>(textureDimensions(surface));
}

@fragment
fn video(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let rgb = video_rgb(source_at(position), 1.5748, vec2<f32>(0.1873, 0.4681), 1.8556);
    return dithered(vec4<f32>(rgb, 1.0) * quad.alpha * coverage(position.xy, quad.clip, quad.clip_radius), position);
}

fn tone_mapped(nits: f32) -> f32 {
    let source = pq_encode(vec3<f32>(quad.color.z)).x;
    let target_peak = pq_encode(vec3<f32>(quad.color.y)).x / source;
    if target_peak >= 1.0 {
        return min(nits, quad.color.y);
    }
    let value = min(pq_encode(vec3<f32>(nits)).x / source, 1.0);
    let knee = max(1.5 * target_peak - 0.5, 0.0);
    if value <= knee {
        return nits;
    }
    let t = (value - knee) / (1.0 - knee);
    let curved = (2.0 * t * t * t - 3.0 * t * t + 1.0) * knee
        + (t * t * t - 2.0 * t * t + t) * (1.0 - knee)
        + (3.0 * t * t - 2.0 * t * t * t) * target_peak;
    return pq_decode(vec3<f32>(curved * source)).x;
}

fn hdr_shown(encoded: vec3<f32>) -> vec3<f32> {
    let nits = pq_decode(encoded);
    let brightest = max(max(nits.r, nits.g), max(nits.b, 0.0001));
    let wide = nits * (tone_mapped(brightest) / brightest) / quad.color.x;
    let linear = vec3<f32>(
        dot(vec3<f32>(1.6604910, -0.5876411, -0.0728499), wide),
        dot(vec3<f32>(-0.1245505, 1.1328999, -0.0083494), wide),
        dot(vec3<f32>(-0.0181508, -0.1005789, 1.1187297), wide),
    );
    let shown = extended_srgb_encode(linear);
    return select(clamp(shown, vec3<f32>(0.0), vec3<f32>(1.0)), shown, quad.color.w > 0.5);
}

@fragment
fn hdr(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = textureSampleLevel(surface, filtered, source_at(position), 0.0);
    let opaque = texel.rgb / max(texel.a, 0.0001);
    let shown = vec4<f32>(hdr_shown(opaque) * texel.a, texel.a);
    return dithered(shown * quad.alpha * coverage(position.xy, quad.clip, quad.clip_radius), position);
}

@fragment
fn hdr_video(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let rgb = video_rgb(source_at(position), 1.4746, vec2<f32>(0.1646, 0.5714), 1.8814);
    let shown = vec4<f32>(hdr_shown(rgb), 1.0);
    return dithered(shown * quad.alpha * coverage(position.xy, quad.clip, quad.clip_radius), position);
}

@fragment
fn solid(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return quad.color * coverage(position.xy, quad.clip, quad.clip_radius);
}

@fragment
fn shadow(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let falloff = box_shadow(position.xy, quad.caster, quad.caster_radius, quad.blur);
    let uncovered = 1.0 - coverage(position.xy, quad.clip, quad.clip_radius);
    let shade = quad.color * (falloff * uncovered * quad.alpha);
    let visible = select(0.0, quad.dither, shade.a > 0.0);
    return vec4<f32>(shade.rgb, clamp(shade.a + noise(position) * visible, 0.0, 1.0));
}

fn sample_at(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(surface, filtered, uv, 0.0);
}

fn to_linear(encoded: vec3<f32>) -> vec3<f32> {
    return pow(encoded, vec3<f32>(2.2));
}

fn to_encoded(linear: vec3<f32>) -> vec3<f32> {
    return pow(max(linear, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

@fragment
fn blur_tone(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(surface));
    let local = (position.xy - quad.origin) / quad.size;
    let source = quad.source_origin + local.x * quad.source_across + local.y * quad.source_down;
    let texel = textureSampleLevel(surface, filtered, source / dimensions, 0.0);
    let weights = vec3<f32>(0.2126, 0.7152, 0.0722);
    let encoded_luma = dot(texel.rgb, weights);
    let saturated = clamp(mix(vec3<f32>(encoded_luma), texel.rgb, quad.color.z), vec3<f32>(0.0), vec3<f32>(1.0));
    var linear = to_linear(saturated);
    let luminance = dot(linear, weights);
    if quad.color.y > 0.5 {
        if luminance < quad.color.x {
            linear = mix(linear, vec3<f32>(1.0), (quad.color.x - luminance) / (1.0 - luminance));
        }
    } else if luminance > quad.color.x {
        linear = linear * quad.color.x / luminance;
    }
    let toned = vec4<f32>(to_encoded(linear), 1.0);
    return dithered(toned * quad.alpha * coverage(position.xy, quad.clip, quad.clip_radius), position);
}

fn srgb_decode(encoded: vec3<f32>) -> vec3<f32> {
    let size = abs(encoded);
    let low = size / 12.92;
    let high = pow((size + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return sign(encoded) * select(high, low, size <= vec3<f32>(0.04045));
}

fn extended_srgb_encode(linear: vec3<f32>) -> vec3<f32> {
    let size = abs(linear);
    let low = size * 12.92;
    let high = 1.055 * pow(size, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return sign(linear) * select(high, low, size <= vec3<f32>(0.0031308));
}

fn pq_encode(nits: vec3<f32>) -> vec3<f32> {
    let power = pow(clamp(nits / 10000.0, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(0.1593017578125));
    return pow((0.8359375 + 18.8515625 * power) / (1.0 + 18.6875 * power), vec3<f32>(78.84375));
}

fn pq_decode(encoded: vec3<f32>) -> vec3<f32> {
    let power = pow(clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 78.84375));
    let ratio = max(power - vec3<f32>(0.8359375), vec3<f32>(0.0)) / (18.8515625 - 18.6875 * power);
    return 10000.0 * pow(ratio, vec3<f32>(1.0 / 0.1593017578125));
}

fn srgb_encode(linear: vec3<f32>) -> vec3<f32> {
    let clamped = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = clamped * 12.92;
    let high = 1.055 * pow(clamped, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(high, low, clamped <= vec3<f32>(0.0031308));
}

@fragment
fn display_colour(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = textureLoad(surface, vec2<i32>(position.xy), 0);
    let linear = srgb_decode(texel.rgb);
    let converted = vec3<f32>(
        dot(quad.color.xyz, linear),
        dot(quad.clip.xyz, linear),
        dot(quad.caster.xyz, linear),
    );
    if quad.alpha > 1.5 {
        return dithered(vec4<f32>(pq_encode(max(converted, vec3<f32>(0.0)) * quad.blur), 1.0), position);
    }
    if quad.alpha < 0.5 {
        return dithered(vec4<f32>(srgb_encode(converted), 1.0), position);
    }
    let width = f32(textureDimensions(chroma).x);
    let at = (sqrt(clamp(converted, vec3<f32>(0.0), vec3<f32>(1.0))) * (width - 1.0) + 0.5) / width;
    let encoded = vec4<f32>(
        textureSampleLevel(chroma, filtered, vec2<f32>(at.r, 0.5), 0.0).r,
        textureSampleLevel(chroma, filtered, vec2<f32>(at.g, 0.5), 0.0).g,
        textureSampleLevel(chroma, filtered, vec2<f32>(at.b, 0.5), 0.0).b,
        1.0,
    );
    return dithered(encoded, position);
}

@fragment
fn blur_down(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = position.xy / quad.size;
    let half_texel = quad.blur * 0.5 / vec2<f32>(textureDimensions(surface));
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
