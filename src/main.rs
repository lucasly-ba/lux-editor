mod buffer;
mod cursor;
mod renderer;

use buffer::Buffer;
use cursor::Cursor;
use renderer::render;

fn main() {
    let buf = Buffer::from_file("test.txt");
    let cursor = Cursor::new();
    render(&buf, &cursor);
}
