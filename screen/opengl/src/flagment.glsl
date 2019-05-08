#version 100

uniform sampler2D texture;
varying mediump vec2 vuv;
// varying vec2 vuv;

void main(void){
    gl_FragColor = texture2D(texture, vuv);
}
