uniform mat4 unif_matrix;
attribute vec2 position;
attribute vec2 uv;
varying vec2 vuv;

void main(void){
    gl_Position = unif_matrix * vec4(position, 0.0, 1.0);
    vuv = uv;
}