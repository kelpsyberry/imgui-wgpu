struct ViewUniform {
    scale: vec2<f32>,
    translate: vec2<f32>,
}

@group(0) @binding(0) var<uniform> view: ViewUniform;

struct VertOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VertOutput {
    var output: VertOutput;
    output.pos = vec4<f32>((pos * view.scale + view.translate) * vec2<f32>(1.0, -1.0), 0.0, 1.0);
    output.uv = uv;
    output.color = color;
    return output;
}

@group(1) @binding(0) var t_texture: texture_2d<f32>;
@group(1) @binding(1) var s_texture: sampler;

@fragment
fn fs_main(
    @location(0) uv: vec2<f32>,
    @location(1) vert_color: vec4<f32>,
) -> @location(0) vec4<f32> {
    var tex_color: vec4<f32> = textureSample(t_texture, s_texture, uv);
    var alpha: f32 = vert_color.a * tex_color.a;
    return vec4<f32>(
        pow(vert_color.rgb * alpha, vec3<f32>(2.2)) * tex_color.rgb,
        1.0 - pow(1.0 - alpha, 2.2)
    );
}
