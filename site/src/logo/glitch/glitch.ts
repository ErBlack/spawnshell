import FRAGMENT from "./glitch.frag.glsl?raw";
import VERTEX from "./glitch.vert.glsl?raw";

const DURATION_MS = 900;
const UNIFORMS = ["u_from", "u_to", "u_time", "u_swap", "u_amount", "u_seed"];

function compileShader(
  gl: WebGLRenderingContext,
  type: number,
  source: string,
): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) throw new Error("couldn't create a shader");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS))
    throw new Error(gl.getShaderInfoLog(shader) ?? "shader didn't compile");
  return shader;
}

function createTexture(gl: WebGLRenderingContext, unit: number): WebGLTexture {
  const texture = gl.createTexture();
  gl.activeTexture(gl.TEXTURE0 + unit);
  gl.bindTexture(gl.TEXTURE_2D, texture);
  for (const parameter of [gl.TEXTURE_WRAP_S, gl.TEXTURE_WRAP_T])
    gl.texParameteri(gl.TEXTURE_2D, parameter, gl.CLAMP_TO_EDGE);
  for (const parameter of [gl.TEXTURE_MIN_FILTER, gl.TEXTURE_MAG_FILTER])
    gl.texParameteri(gl.TEXTURE_2D, parameter, gl.LINEAR);
  return texture;
}

export class Glitch {
  private readonly gl: WebGLRenderingContext;
  private readonly uniforms: Record<string, WebGLUniformLocation | null>;
  private readonly textures: [WebGLTexture, WebGLTexture];

  private constructor(gl: WebGLRenderingContext) {
    this.gl = gl;
    const program = gl.createProgram();
    gl.attachShader(program, compileShader(gl, gl.VERTEX_SHADER, VERTEX));
    gl.attachShader(program, compileShader(gl, gl.FRAGMENT_SHADER, FRAGMENT));
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS))
      throw new Error(gl.getProgramInfoLog(program) ?? "program didn't link");
    gl.useProgram(program);

    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(
      gl.ARRAY_BUFFER,
      new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]),
      gl.STATIC_DRAW,
    );
    const unit = gl.getAttribLocation(program, "a_unit");
    gl.enableVertexAttribArray(unit);
    gl.vertexAttribPointer(unit, 2, gl.FLOAT, false, 0, 0);

    this.uniforms = Object.fromEntries(
      UNIFORMS.map((name) => [name, gl.getUniformLocation(program, name)]),
    );
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
    this.textures = [createTexture(gl, 0), createTexture(gl, 1)];
    gl.uniform1i(this.uniforms.u_from, 0);
    gl.uniform1i(this.uniforms.u_to, 1);
  }

  static create(canvas: HTMLCanvasElement): Glitch | undefined {
    const gl = canvas.getContext("webgl", { premultipliedAlpha: true });
    if (!gl) return undefined;
    try {
      return new Glitch(gl);
    } catch (error) {
      console.error(error);
      return undefined;
    }
  }

  play(
    from: HTMLCanvasElement,
    to: HTMLCanvasElement,
    onLevel: (level: number) => void,
  ): Promise<void> {
    const gl = this.gl;
    const uniforms = this.uniforms;
    this.upload(0, from);
    this.upload(1, to);
    gl.uniform1f(uniforms.u_seed, Math.random() * 1000);
    const start = performance.now();

    return new Promise((resolve) => {
      const frame = (now: number) => {
        const t = Math.min(1, (now - start) / DURATION_MS);
        const amount = Math.sin(Math.PI * t) ** 0.7;
        gl.uniform1f(uniforms.u_time, (now - start) / 1000);
        const swap = Math.min(1, Math.max(0, (t - 0.3) / 0.4));
        gl.uniform1f(uniforms.u_swap, swap * swap * (3 - 2 * swap));
        gl.uniform1f(uniforms.u_amount, amount);
        onLevel(amount);
        gl.viewport(0, 0, gl.canvas.width, gl.canvas.height);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        if (t < 1) requestAnimationFrame(frame);
        else resolve();
      };
      requestAnimationFrame(frame);
    });
  }

  private upload(unit: 0 | 1, source: HTMLCanvasElement) {
    const gl = this.gl;
    gl.activeTexture(gl.TEXTURE0 + unit);
    gl.bindTexture(gl.TEXTURE_2D, this.textures[unit]);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
  }
}
