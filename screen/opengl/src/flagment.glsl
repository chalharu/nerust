uniform sampler2D texture;
varying vec2 vuv;

void main(void){
    gl_FragColor = texture2D(texture, vuv);
}
