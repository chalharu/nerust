#version 150

uniform sampler2D screen_texture;
in vec2 vuv;
out vec4 frag_color;

void main(void){
    frag_color = texture(screen_texture, vuv);
}
