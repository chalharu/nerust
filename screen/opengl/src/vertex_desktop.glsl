#version 150

uniform mat4 unif_matrix;
in vec2 position;
in vec2 uv;
out vec2 vuv;

void main(void){
    gl_Position = unif_matrix * vec4(position, 0.0, 1.0);
    vuv = uv;
}
