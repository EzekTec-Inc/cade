use pulldown_cmark::{Parser, Event, Tag, TagEnd};

fn main() {
    let p = Parser::new("test");
    for e in p {
        match e {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {},
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {},
                _ => {}
            },
            _ => {}
        }
    }
}
