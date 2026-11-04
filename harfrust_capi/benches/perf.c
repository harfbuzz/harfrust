/* Standalone C API microbenchmarks. Build after cargo build -p harfrust_capi --release:
 *   cc -O3 harfrust_capi/benches/perf.c -o /tmp/hr-capi-perf  # Linux: add -ldl
 *   /tmp/hr-capi-perf target/release/libharfrust_c.dylib /path/to/font.ttf MODE
 * Use .so on Linux. Each result is the minimum of five warm trials, in ms.
 * Modes: input16-long, input16-short, input8-long, nominal, nominal-ja,
 *        metrics-batch, metrics-scalar, shape-short-one, shape-short-four,
 *        shape-short-ja-one, shape-short-ja-four, shape-long, shape-long-nokern.
 * Long inputs contain 1.8M items. A trial runs 8 input/nominal batches,
 * 4 metrics batches, 100K short shapes (8 characters), or 3 long shapes.
 * Japanese modes need a font covering the Japanese text in main().
 */
#define _POSIX_C_SOURCE 200809L
#include <assert.h>
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "../include/hr.h"
static __typeof__(&hr_blob_create_from_file) p_hr_blob_create_from_file;
static __typeof__(&hr_face_create) p_hr_face_create;
static __typeof__(&hr_font_create) p_hr_font_create;
static __typeof__(&hr_buffer_create) p_hr_buffer_create;
static __typeof__(&hr_blob_destroy) p_hr_blob_destroy;
static __typeof__(&hr_face_destroy) p_hr_face_destroy;
static __typeof__(&hr_font_destroy) p_hr_font_destroy;
static __typeof__(&hr_buffer_destroy) p_hr_buffer_destroy;
static __typeof__(&hr_buffer_clear_contents) p_hr_buffer_clear_contents;
static __typeof__(&hr_buffer_set_direction) p_hr_buffer_set_direction;
static __typeof__(&hr_buffer_set_script) p_hr_buffer_set_script;
static __typeof__(&hr_buffer_add_utf16) p_hr_buffer_add_utf16;
static __typeof__(&hr_buffer_add_latin1) p_hr_buffer_add_latin1;
static __typeof__(&hr_buffer_get_length) p_hr_buffer_get_length;
static __typeof__(&hr_font_get_nominal_glyphs) p_hr_font_get_nominal_glyphs;
static __typeof__(&hr_font_get_glyph_h_advance) p_hr_font_get_glyph_h_advance;
static __typeof__(&hr_font_get_glyph_h_advances) p_hr_font_get_glyph_h_advances;
static __typeof__(&hr_shape) p_hr_shape;
static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec * 1e-9; }
static volatile uint64_t checksum;
int main(int argc, char **argv) {
 if (argc != 4) {
   fprintf(stderr, "Usage: %s LIBRARY FONT MODE\n", argv[0]);
   return 1;
 }
 void *lib = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL); if (!lib) { puts(dlerror()); return 1; }
#define LOAD(n) do { *(void **)(&n) = dlsym(lib, &(#n)[2]); assert(n); } while (0)
 LOAD(p_hr_blob_create_from_file); LOAD(p_hr_face_create); LOAD(p_hr_font_create); LOAD(p_hr_buffer_create);
 LOAD(p_hr_blob_destroy); LOAD(p_hr_face_destroy); LOAD(p_hr_font_destroy); LOAD(p_hr_buffer_destroy);
 LOAD(p_hr_buffer_clear_contents); LOAD(p_hr_buffer_set_direction); LOAD(p_hr_buffer_set_script);
 LOAD(p_hr_buffer_add_utf16); LOAD(p_hr_buffer_add_latin1); LOAD(p_hr_buffer_get_length);
 LOAD(p_hr_font_get_nominal_glyphs); LOAD(p_hr_font_get_glyph_h_advance); LOAD(p_hr_font_get_glyph_h_advances); LOAD(p_hr_shape);
 struct hr_blob_t *blob = p_hr_blob_create_from_file(argv[2]);
 struct hr_face_t *face = p_hr_face_create(blob, 0);
 struct hr_font_t *font = p_hr_font_create(face);
 struct hr_buffer_t *buf = p_hr_buffer_create();
 const int n = 1800000;
 uint16_t *u16 = malloc((size_t)n * 2); uint8_t *u8 = malloc(n);
 uint32_t *cp = malloc((size_t)n * 4), *gids = malloc((size_t)n * 4); int *adv = malloc((size_t)n * 4);
 const uint16_t ja[] = {0x4eca,0x65e5,0x306f,0x30ab,0x30bf,0x30ab,0x30ca,0x3002};
 int japanese = strstr(argv[3], "ja") != NULL;
 for (int i = 0; i < n; i++) { u8[i] = "The quick brown fox jumps over the lazy dog. "[i%(sizeof("The quick brown fox jumps over the lazy dog. ")-1)]; u16[i] = japanese ? ja[i%8] : u8[i]; cp[i] = u16[i]; }
 double best = 1e99;
 for (int trial = 0; trial < 6; trial++) {
   double t = now();
   if (strstr(argv[3], "input16-long")) {
     for (int r=0;r<8;r++) { p_hr_buffer_clear_contents(buf); p_hr_buffer_add_utf16(buf, u16, n, 0, n); assert(p_hr_buffer_get_length(buf)==(unsigned)n); }
   } else if (strstr(argv[3], "input16-short")) {
     for(int r=0;r<200000;r++) { p_hr_buffer_clear_contents(buf); p_hr_buffer_add_utf16(buf,u16,n,100,8); }
   } else if (strstr(argv[3], "input8-long")) {
     for (int r=0;r<8;r++) { p_hr_buffer_clear_contents(buf); p_hr_buffer_add_latin1(buf, u8, n, 0, n); }
   } else if (strstr(argv[3], "nominal")) {
     for(int r=0;r<8;r++) { unsigned done=p_hr_font_get_nominal_glyphs(font,n,cp,4,gids,4); assert(done==(unsigned)n); checksum+=gids[n-1]; }
   } else if (strstr(argv[3], "metrics")) {
     assert(p_hr_font_get_nominal_glyphs(font,n,cp,4,gids,4)==(unsigned)n);
     t=now();
     for(int r=0;r<4;r++) {
       if(strstr(argv[3],"scalar")) for(int i=0;i<n;i++) adv[i]=p_hr_font_get_glyph_h_advance(font,gids[i]);
       else p_hr_font_get_glyph_h_advances(font,n,gids,4,adv,4);
       checksum+=adv[n-1];
     }
   } else if (strstr(argv[3], "shape-short")) {
     struct hr_feature_t f[4]={{0x6b65726e,1,0,~0u},{0x6c696761,1,0,~0u},{0x63687773,1,0,~0u},{0x68616c74,1,0,~0u}};
     unsigned nf=strstr(argv[3], "four") ? 4 : strstr(argv[3], "one") ? 1 : 0;
     for(int r=0;r<100000;r++) {
       p_hr_buffer_clear_contents(buf); p_hr_buffer_add_utf16(buf,u16,n,100,8);
       p_hr_buffer_set_direction(buf,HR_DIRECTION_LTR); p_hr_buffer_set_script(buf,japanese ? HR_SCRIPT_HAN : HR_SCRIPT_LATIN);
       p_hr_shape(font,buf,f,nf); checksum+=p_hr_buffer_get_length(buf);
     }
   } else if (strstr(argv[3], "shape-long")) {
     struct hr_feature_t f={0x6b65726e,0,0,~0u};
     for(int r=0;r<3;r++) {
       p_hr_buffer_clear_contents(buf); p_hr_buffer_add_latin1(buf,u8,n,0,n);
       p_hr_buffer_set_direction(buf,HR_DIRECTION_LTR); p_hr_buffer_set_script(buf,HR_SCRIPT_LATIN);
       p_hr_shape(font,buf,&f,strstr(argv[3], "nokern") ? 1 : 0); checksum+=p_hr_buffer_get_length(buf);
     }
   } else { fprintf(stderr, "Unknown mode: %s\n", argv[3]); return 1; }
   double elapsed=now()-t; if(trial && elapsed<best) best=elapsed;
 }
 printf("%s: %.3f ms (checksum %llu)\n",argv[3],best*1000,(unsigned long long)checksum);
 p_hr_buffer_destroy(buf); p_hr_font_destroy(font); p_hr_face_destroy(face); p_hr_blob_destroy(blob);
 free(u16);free(u8);free(cp);free(gids);free(adv);dlclose(lib);
}
