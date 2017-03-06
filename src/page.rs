/// A page, can be a blog post or a basic page
use std::cmp::Ordering;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::result::Result as StdResult;


use pulldown_cmark as cmark;
use regex::Regex;
use tera::{Tera, Context};
use serde::ser::{SerializeStruct, self};
use slug::slugify;

use errors::{Result, ResultExt};
use config::Config;
use front_matter::{FrontMatter};


lazy_static! {
    static ref PAGE_RE: Regex = Regex::new(r"^\n?\+\+\+\n((?s).*(?-s))\+\+\+\n((?s).*(?-s))$").unwrap();
}


#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Page {
    /// .md filepath, excluding the content/ bit
    #[serde(skip_serializing)]
    pub filepath: String,
    /// The name of the .md file
    #[serde(skip_serializing)]
    pub filename: String,
    /// The directories above our .md file are called sections
    /// for example a file at content/kb/solutions/blabla.md will have 2 sections:
    /// `kb` and `solutions`
    #[serde(skip_serializing)]
    pub sections: Vec<String>,
    /// The actual content of the page, in markdown
    #[serde(skip_serializing)]
    pub raw_content: String,
    /// The HTML rendered of the page
    pub content: String,
    /// The front matter meta-data
    pub meta: FrontMatter,

    /// The slug of that page.
    /// First tries to find the slug in the meta and defaults to filename otherwise
    pub slug: String,
    /// The relative URL of the page
    pub url: String,
    /// The full URL for that page
    pub permalink: String,

    /// The previous page, by date
    pub previous: Option<Box<Page>>,
    /// The next page, by date
    pub next: Option<Box<Page>>,
}


impl Page {
    pub fn new(meta: FrontMatter) -> Page {
        Page {
            filepath: "".to_string(),
            filename: "".to_string(),
            sections: vec![],
            raw_content: "".to_string(),
            content: "".to_string(),
            slug: "".to_string(),
            url: "".to_string(),
            permalink: "".to_string(),
            meta: meta,
            previous: None,
            next: None,
        }
    }

    // Get word count and estimated reading time
    pub fn get_reading_analytics(&self) -> (usize, usize) {
        // Only works for latin language but good enough for a start
        let word_count: usize = self.raw_content.split_whitespace().count();

        // https://help.medium.com/hc/en-us/articles/214991667-Read-time
        // 275 seems a bit too high though
        (word_count, (word_count / 200))
    }

    // Parse a page given the content of the .md file
    // Files without front matter or with invalid front matter are considered
    // erroneous
    pub fn parse(filepath: &str, content: &str, config: &Config) -> Result<Page> {
        // 1. separate front matter from content
        if !PAGE_RE.is_match(content) {
            bail!("Couldn't find front matter in `{}`. Did you forget to add `+++`?", filepath);
        }

        // 2. extract the front matter and the content
        let caps = PAGE_RE.captures(content).unwrap();
        // caps[0] is the full match
        let front_matter = &caps[1];
        let content = &caps[2];

        // 3. create our page, parse front matter and assign all of that
        let meta = FrontMatter::parse(&front_matter)
            .chain_err(|| format!("Error when parsing front matter of file `{}`", filepath))?;

        let mut page = Page::new(meta);
        page.filepath = filepath.to_string();
        page.raw_content = content.to_string();
        page.content = {
            let mut html = String::new();
            let parser = cmark::Parser::new(&page.raw_content);
            cmark::html::push_html(&mut html, parser);
            html
        };
        let path = Path::new(filepath);
        page.filename = path.file_stem().expect("Couldn't get filename").to_string_lossy().to_string();
        page.slug = {
            if let Some(ref slug) = page.meta.slug {
                slug.to_string()
            } else {
                slugify(page.filename.clone())
            }
        };


        // 4. Find sections
        // Pages with custom urls exists outside of sections
        if let Some(ref u) = page.meta.url {
            page.url = u.to_string();
        } else {
            // find out if we have sections
            for section in path.parent().unwrap().components() {
                page.sections.push(section.as_ref().to_string_lossy().to_string());
            }

            if !page.sections.is_empty() {
                page.url = format!("{}/{}", page.sections.join("/"), page.slug);
            } else {
                page.url = format!("{}", page.slug);
            }
        }
        page.permalink = if config.base_url.ends_with("/") {
            format!("{}{}", config.base_url, page.url)
        } else {
            format!("{}/{}", config.base_url, page.url)
        };

        Ok(page)
    }

    pub fn from_file<P: AsRef<Path>>(path: P, config: &Config) -> Result<Page> {
        let path = path.as_ref();

        let mut content = String::new();
        File::open(path)
            .chain_err(|| format!("Failed to open '{:?}'", path.display()))?
            .read_to_string(&mut content)?;

        // Remove the content string from name
        // Maybe get a path as an arg instead and use strip_prefix?
        Page::parse(&path.strip_prefix("content").unwrap().to_string_lossy(), &content, config)
    }

    fn get_layout_name(&self) -> String {
        match self.meta.layout {
            Some(ref l) => l.to_string(),
            None => "page.html".to_string()
        }
    }

    /// Renders the page using the default layout, unless specified in front-matter
    pub fn render_html(&self, tera: &Tera, config: &Config) -> Result<String> {
        let tpl = self.get_layout_name();
        let mut context = Context::new();
        context.add("site", config);
        context.add("page", self);

        tera.render(&tpl, &context)
            .chain_err(|| "Error while rendering template")
    }
}

impl ser::Serialize for Page {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error> where S: ser::Serializer {
        let mut state = serializer.serialize_struct("page", 13)?;
        state.serialize_field("content", &self.content)?;
        state.serialize_field("title", &self.meta.title)?;
        state.serialize_field("description", &self.meta.description)?;
        state.serialize_field("date", &self.meta.date)?;
        state.serialize_field("slug", &self.slug)?;
        state.serialize_field("url", &format!("/{}", self.url))?;
        state.serialize_field("permalink", &self.permalink)?;
        state.serialize_field("tags", &self.meta.tags)?;
        state.serialize_field("draft", &self.meta.draft)?;
        state.serialize_field("category", &self.meta.category)?;
        state.serialize_field("extra", &self.meta.extra)?;
        let (word_count, reading_time) = self.get_reading_analytics();
        state.serialize_field("word_count", &word_count)?;
        state.serialize_field("reading_time", &reading_time)?;
        state.end()
    }
}

impl PartialOrd for Page {
    fn partial_cmp(&self, other: &Page) -> Option<Ordering> {
        if self.meta.date.is_none() {
            println!("No self data");
            return Some(Ordering::Less);
        }

        if other.meta.date.is_none() {
            println!("No other date");
            return Some(Ordering::Greater);
        }

        let this_date = self.meta.parse_date().unwrap();
        let other_date = other.meta.parse_date().unwrap();

        if this_date > other_date {
            return Some(Ordering::Less);
        }
        if this_date < other_date {
            return Some(Ordering::Greater);
        }

        Some(Ordering::Equal)
    }
}


#[cfg(test)]
mod tests {
    use super::{Page};
    use config::Config;


    #[test]
    fn test_can_parse_a_valid_page() {
        let content = r#"
+++
title = "Hello"
description = "hey there"
slug = "hello-world"
+++
Hello world"#;
        let res = Page::parse("post.md", content, &Config::default());
        assert!(res.is_ok());
        let page = res.unwrap();

        assert_eq!(page.meta.title, "Hello".to_string());
        assert_eq!(page.meta.slug.unwrap(), "hello-world".to_string());
        assert_eq!(page.raw_content, "Hello world".to_string());
        assert_eq!(page.content, "<p>Hello world</p>\n".to_string());
    }

    #[test]
    fn test_can_find_one_parent_directory() {
        let content = r#"
+++
title = "Hello"
description = "hey there"
slug = "hello-world"
+++
Hello world"#;
        let res = Page::parse("posts/intro.md", content, &Config::default());
        assert!(res.is_ok());
        let page = res.unwrap();
        assert_eq!(page.sections, vec!["posts".to_string()]);
    }

    #[test]
    fn test_can_find_multiple_parent_directories() {
        let content = r#"
+++
title = "Hello"
description = "hey there"
slug = "hello-world"
+++
Hello world"#;
        let res = Page::parse("posts/intro/start.md", content, &Config::default());
        assert!(res.is_ok());
        let page = res.unwrap();
        assert_eq!(page.sections, vec!["posts".to_string(), "intro".to_string()]);
    }

    #[test]
    fn test_can_make_url_from_sections_and_slug() {
        let content = r#"
+++
title = "Hello"
description = "hey there"
slug = "hello-world"
+++
Hello world"#;
        let mut conf = Config::default();
        conf.base_url = "http://hello.com/".to_string();
        let res = Page::parse("posts/intro/start.md", content, &conf);
        assert!(res.is_ok());
        let page = res.unwrap();
        assert_eq!(page.url, "posts/intro/hello-world");
        assert_eq!(page.permalink, "http://hello.com/posts/intro/hello-world");
    }

    #[test]
    fn test_can_make_permalink_with_non_trailing_slash_base_url() {
        let content = r#"
+++
title = "Hello"
description = "hey there"
slug = "hello-world"
+++
Hello world"#;
        let mut conf = Config::default();
        conf.base_url = "http://hello.com".to_string();
        let res = Page::parse("posts/intro/start.md", content, &conf);
        assert!(res.is_ok());
        let page = res.unwrap();
        assert_eq!(page.url, "posts/intro/hello-world");
        println!("{}", page.permalink);
        assert_eq!(page.permalink, format!("{}{}", conf.base_url, "/posts/intro/hello-world"));
    }

    #[test]
    fn test_can_make_url_from_slug_only() {
        let content = r#"
+++
title = "Hello"
description = "hey there"
slug = "hello-world"
+++
Hello world"#;
        let res = Page::parse("start.md", content, &Config::default());
        assert!(res.is_ok());
        let page = res.unwrap();
        assert_eq!(page.url, "hello-world");
        assert_eq!(page.permalink, format!("{}{}", Config::default().base_url, "hello-world"));
    }

    #[test]
    fn test_errors_on_invalid_front_matter_format() {
        let content = r#"
title = "Hello"
description = "hey there"
slug = "hello-world"
+++
Hello world"#;
        let res = Page::parse("start.md", content, &Config::default());
        assert!(res.is_err());
    }

    #[test]
    fn test_can_make_slug_from_non_slug_filename() {
        let content = r#"
+++
title = "Hello"
description = "hey there"
+++
Hello world"#;
        let res = Page::parse("file with space.md", content, &Config::default());
        assert!(res.is_ok());
        let page = res.unwrap();
        assert_eq!(page.slug, "file-with-space");
        assert_eq!(page.permalink, format!("{}{}", Config::default().base_url, "file-with-space"));
    }

    #[test]
    fn test_reading_analytics_short() {
        let content = r#"
+++
title = "Hello"
description = "hey there"
+++
Hello world"#;
        let res = Page::parse("file with space.md", content, &Config::default());
        assert!(res.is_ok());
        let page = res.unwrap();
        let (word_count, reading_time) = page.get_reading_analytics();
        assert_eq!(word_count, 2);
        assert_eq!(reading_time, 0);
    }

    #[test]
    fn test_reading_analytics_long() {
        let mut content = r#"
+++
title = "Hello"
description = "hey there"
+++
Hello world"#.to_string();
        for _ in 0..1000 {
            content.push_str(" Hello world");
        }
        let res = Page::parse("hello.md", &content, &Config::default());
        assert!(res.is_ok());
        let page = res.unwrap();
        let (word_count, reading_time) = page.get_reading_analytics();
        assert_eq!(word_count, 2002);
        assert_eq!(reading_time, 10);
    }
}
