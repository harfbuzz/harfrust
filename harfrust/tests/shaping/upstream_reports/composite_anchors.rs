use crate::shape;

#[test]
fn case_000() {
    assert_eq!(shape("tests/fonts/in-house/27b2c982e9b2f1c44b2ca0aa5cb5abc830d5181b.ttf", "\u{0069}\u{0078}\u{0079}\u{007A}", "--variations=wght=0 --font-size=1000 --show-extents"), "[inner=0+1000<0,400,200,-400>|outer=1+1000<0,400,400,-700>|double=2+1000<-200,450,900,-950>|shifted=3+1000<0,500,400,-800>]");
}

#[test]
fn case_001() {
    assert_eq!(shape("tests/fonts/in-house/27b2c982e9b2f1c44b2ca0aa5cb5abc830d5181b.ttf", "\u{0069}\u{0078}\u{0079}\u{007A}", "--variations=wght=0 --font-size=2000 --show-extents"), "[inner=0+2000<0,800,400,-800>|outer=1+2000<0,800,800,-1400>|double=2+2000<-400,900,1800,-1900>|shifted=3+2000<0,1000,800,-1600>]");
}

#[test]
fn case_002() {
    assert_eq!(shape("tests/fonts/in-house/27b2c982e9b2f1c44b2ca0aa5cb5abc830d5181b.ttf", "\u{0069}\u{0078}\u{0079}\u{007A}", "--variations=wght=0.5 --font-size=1000 --show-extents"), "[inner=0+1000<0,400,200,-400>|outer=1+1000<0,400,400,-700>|double=2+1000<-200,450,900,-950>|shifted=3+1000<0,500,400,-800>]");
}

#[test]
fn case_003() {
    assert_eq!(shape("tests/fonts/in-house/27b2c982e9b2f1c44b2ca0aa5cb5abc830d5181b.ttf", "\u{0069}\u{0078}\u{0079}\u{007A}", "--variations=wght=0.5 --font-size=2000 --show-extents"), "[inner=0+2000<0,800,400,-800>|outer=1+2000<0,800,800,-1400>|double=2+2000<-400,900,1800,-1900>|shifted=3+2000<0,1000,800,-1600>]");
}

#[test]
fn case_004() {
    assert_eq!(shape("tests/fonts/in-house/27b2c982e9b2f1c44b2ca0aa5cb5abc830d5181b.ttf", "\u{0069}\u{0078}\u{0079}\u{007A}", "--variations=wght=1 --font-size=1000 --show-extents"), "[inner=0+1000<0,400,200,-400>|outer=1+1000<0,400,400,-700>|double=2+1000<-200,450,900,-950>|shifted=3+1000<0,500,400,-800>]");
}

#[test]
fn case_005() {
    assert_eq!(shape("tests/fonts/in-house/27b2c982e9b2f1c44b2ca0aa5cb5abc830d5181b.ttf", "\u{0069}\u{0078}\u{0079}\u{007A}", "--variations=wght=1 --font-size=2000 --show-extents"), "[inner=0+2000<0,800,400,-800>|outer=1+2000<0,800,800,-1400>|double=2+2000<-400,900,1800,-1900>|shifted=3+2000<0,1000,800,-1600>]");
}
