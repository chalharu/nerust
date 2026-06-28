void main(void) {
    gl_FragColor = vec4(decoded_rgb_for_output(output_coords()), 1.0);
}
