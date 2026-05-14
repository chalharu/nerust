#version 120

uniform sampler2D screen_texture;
varying vec2 vuv;

void main(void){
    gl_FragColor = texture2D(screen_texture, vuv);
}
