precision highp float;
uniform sampler2D u_from;
uniform sampler2D u_to;
uniform float u_time;
uniform float u_swap;
uniform float u_amount;
uniform float u_seed;
varying vec2 v_uv;

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

float hash1(float x) {
    return hash(vec2(x, 17.31));
}

vec4 picture(vec2 uv, float swapped) {
    return mix(texture2D(u_from, uv), texture2D(u_to, uv), swapped);
}

void main() {
    vec2 uv = v_uv;

    float shakeSlot = floor(u_time * 30.0) + u_seed;
    uv += (vec2(hash1(shakeSlot), hash1(shakeSlot + 7.0)) - 0.5) * u_amount * 0.02;

    float flickerSlot = floor(u_time * 12.0) + u_seed;
    float flickerOn = step(1.0 - u_amount, hash1(flickerSlot + 1.0));
    float bandHeight = mix(0.003, 0.05, hash1(flickerSlot + 3.0));
    float band = step(abs(uv.y - hash1(flickerSlot + 5.0)), bandHeight);
    uv.x += flickerOn * band * (hash1(flickerSlot + 9.0) - 0.5) * 0.1;

    float tearSlot = floor(u_time * 8.0) + u_seed;
    float row = floor(uv.y * mix(8.0, 48.0, hash1(tearSlot + 2.0)));
    float torn = step(1.0 - u_amount * 0.6, hash(vec2(row, tearSlot)));
    uv.x += torn * (hash(vec2(row, tearSlot + 1.0)) - 0.5) * 0.35 * u_amount;

    float blockSlot = floor(u_time * 10.0) + u_seed;
    vec2 block = floor(uv * vec2(16.0, 9.0) * mix(0.5, 2.5, hash1(blockSlot + 4.0)));
    float blockOn = step(1.0 - u_amount * 0.5, hash(block + blockSlot));
    uv += blockOn * (vec2(hash(block + blockSlot + 1.0), hash(block + blockSlot + 2.0)) - 0.5) * 0.25 * u_amount;

    float swapped = step(hash(floor(v_uv * vec2(24.0, 14.0)) + 91.0), u_swap);
    float split = u_amount * 0.02 * (0.4 + hash1(floor(u_time * 20.0) + 15.0));
    vec4 red = picture(uv + vec2(split, 0.0), swapped);
    vec4 green = picture(uv, swapped);
    vec4 blue = picture(uv - vec2(split, 0.0), swapped);
    vec4 color = vec4(red.r, green.g, blue.b, max(max(red.a, green.a), blue.a));

    color.rgb = mix(color.rgb, color.gbr, blockOn * u_amount * step(0.5, hash(block + blockSlot + 3.0)));
    color *= 1.0 - u_amount * 0.35 * step(0.5, fract(gl_FragCoord.y * 0.5));

    gl_FragColor = color;
}
