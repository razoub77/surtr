//! Release Filename parsing: turning a played file's path into a clean search
//! query, a season marker and a Parsed Episode — or a clean rejection.
//!
//! Pure functions only: no filesystem, no network. The Daemon Core asks twice
//! per loaded file (the Anime Root gate, then the parser), and the matcher
//! feeds the resulting query to AniList. The same season-marker grammar is exposed
//! as [`title_season`] so a Match's titles can corroborate the marker.
//!
//! The path is reduced to words: caseless, accents folded, `_`, `-` and `.`
//! treated as spaces. Bracket tags, checksums and release-group words are
//! dropped in one pass (`clean_tokens`); parentheses keep their words unless
//! they are a year or a technical tag phrase. The season marker, the Movie
//! marker and the episode marker are then read off the ends, and a trailing
//! year is dropped. The parent folder supplies the title when the basename
//! carries none, and a basename that names the show keeps the folder's own
//! spelling as a last-resort query — AniList search finds `Dai Seijo`, not the
//! glued `Daiseijo` a release filename writes.

/// What a played path says about the anime it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRelease {
    /// Clean title, without season marker or year. For a season above 1 it
    /// ends in `season N` (`youjo senki season 2`): the form the matcher
    /// appends and `overrides.json` keys use.
    pub title: String,
    /// Season marker from the filename, else the immediate parent folder.
    /// `None` means unmarked, which matching treats as season 1.
    pub season: Option<u32>,
    /// The Parsed Episode; `None` means the file is a Movie.
    pub episode: Option<u32>,
    /// A Movie: no Parsed Episode, so no Episode segment in the Presence.
    pub movie: bool,
    /// Release year read off the basename, else the nearest ancestor folder:
    /// a constraint on the Match, never a query token. An ancestor folder is
    /// scanned whole (`Show.2019` counts); only the basename loses its
    /// extension first.
    pub year: Option<u32>,
    /// The words after the last `-` of a Movie filename (`Sword Art Online -
    /// Movie 01 - Ordinal Scale` → `ordinal scale`): the fallback query when
    /// the full movie title returns no Candidates.
    pub tail: Option<String>,
    /// The parent folder's title, when the basename named the show itself
    /// (`[ASW] Daiseijo wa … - 05` under a folder spelling it `Dai Seijo wa …`).
    /// The fallback query when the basename's title returns zero Candidates;
    /// `None` when the folder *is* the title, or the basename is title-less.
    pub folder: Option<String>,
}

/// Why a path yields no Presence at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejected {
    /// Not a local filesystem path (URL, stream).
    NotLocal,
    /// No filename component.
    NoFilename,
    /// Neither the basename nor the parent folder carries a title.
    NoTitle,
    /// An absolute-numbered file tagged `[12]` with no season marker: which
    /// season the number belongs to is season resolution.
    UnresolvedEpisode,
    /// Batch range, multi-episode file, OVA, or an unbalanced tag.
    Unsupported(&'static str),
}

/// The folder/filename seam inside one token stream.
const SEAM: &str = ":";

/// Parse a played path into a Release, or a reason it yields no Presence.
pub fn parse(path: &str) -> Result<ParsedRelease, Rejected> {
    if !is_local(path) {
        return Err(Rejected::NotLocal);
    }
    let raw = path.trim_end_matches(['.', ' ']);
    let raw_name = raw.rsplit(['\\', '/']).next().unwrap_or(raw);
    let name = stem(raw_name);
    if name.is_empty() {
        return Err(Rejected::NoFilename);
    }
    // A year never reaches AniList search (it makes it return zero). The
    // basename is primary; a file that omits it inherits the nearest ancestor
    // folder's year, e.g. `Fruits Basket (2019)\Season 1\Show - 01.mkv`.
    let year = year_of(&name).or_else(|| ancestor_year(raw));
    // `S01OVA-…`: a special with no numeric episode. It parses (the folder
    // still names the show) but is neither a Movie nor an episode, so it
    // yields no Presence.
    let ova = name.to_ascii_lowercase().starts_with("s01ova");

    let file = clean_tokens(&name)?;
    // `Show - 50 [12]`: the tag says the number continues a franchise, so a
    // season-1 query would be wrong. Season resolution owns that call.
    if !file.is_empty() && file.iter().all(|t| t.bracketed) {
        return Err(Rejected::UnresolvedEpisode);
    }

    let folder = parent(raw)
        .map(|folder| clean_tokens(&stem(&folder)))
        .transpose()?;
    let mut tokens: Vec<String> = folder
        .map(|folder| folder.into_iter().map(|t| t.text).collect())
        .unwrap_or_default();
    tokens.push(SEAM.to_owned());
    tokens.extend(file.into_iter().map(|t| t.text));

    let (tokens, season) = split_season(tokens);
    // A basename made of markers only (`S02E05`, `S02E05 - …`) is title-less:
    // the folder supplies the title. Episode-title words after `SxxExx` are
    // not the show's title, so only the run before the episode marker counts.
    let file_tokens: Vec<&String> = tokens.iter().skip_while(|t| *t != SEAM).skip(1).collect();
    let before_episode: Vec<&String> = file_tokens
        .iter()
        .take_while(|t| sxxexx(t).is_none() && !is_number(t))
        .copied()
        .collect();
    let bare = ova || before_episode.iter().all(|t| is_episode_metadata(t));
    let (tokens, is_movie) = strip_movie_marker(tokens);
    let (mut tokens, episode) = split_episode(tokens)?;
    // A year never reaches the query (it makes search return zero). A year
    // also takes the tag remnant beside it (`Kaguya 2026 1`, `… 2023 -`).
    let mut i = 0;
    while i < tokens.len() {
        if is_year(&tokens[i]) {
            let start = if i > 0 && tokens[i - 1] == "-" {
                i - 1
            } else {
                i
            };
            let mut end = i + 1;
            while tokens
                .get(end)
                .is_some_and(|t| t == "-" || (is_number(t) && t.len() <= 2))
            {
                end += 1;
            }
            tokens.drain(start..end.min(tokens.len()));
            i = start;
            continue;
        }
        i += 1;
    }
    // `Show - [OAD]`: a `-` that ends the filename with no number is not a
    // Movie. A group tail (`x265-EMBER`) glues its `-` to the preceding word,
    // so only a spaced separator with nothing after it is a missing episode.
    if episode.is_none()
        && !is_movie
        && name.contains(" - ")
        && tokens.last().is_some_and(|t| t == "-")
    {
        return Err(Rejected::Unsupported("no episode"));
    }

    // Before the seam is the folder: the title only when the basename had
    // none. Otherwise the basename names the show and the folder repeats it.
    let is_movie = !ova && (is_movie || episode.is_none());
    // `Show - Movie 01 - Tail`: when the full movie title finds no Candidate,
    // the run after the last `-` of the filename is the fallback query. It
    // comes from the same final token stream the title was joined from, so it
    // is already free of tags, years and season markers.
    let tail = if is_movie { movie_tail(&tokens, seam_position(&tokens)) } else { None };
    let seam = seam_position(&tokens);
    // A basename that names the show leaves the folder as a second spelling of
    // the same title: `Daiseijo` in the file, `Dai Seijo` in the folder. It is
    // a fallback query only when that spelling differs.
    let folder = match (seam, bare) {
        (Some(seam), false) => Some(join_title(&strip_season_words(&tokens[..seam]))),
        _ => None,
    };
    let title_tokens = match (seam, bare) {
        (Some(seam), true) => tokens[..seam].to_vec(),
        (Some(seam), false) => tokens[seam + 1..].to_vec(),
        (None, _) => tokens,
    };
    let title = join_title(&strip_season_words(&title_tokens));
    // A single letter is a drive or a junk folder (`D:\A\…`), never a title.
    if title.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        return Err(Rejected::NoTitle);
    }
    let folder = folder.filter(|folder| {
        let words = folder.chars().filter(|c| c.is_alphanumeric()).count();
        words >= 2 && crate::scorer::normalize(folder) != crate::scorer::normalize(&title)
    });

    Ok(ParsedRelease {
        title: with_season(title, season),
        season,
        episode,
        movie: is_movie,
        year,
        tail,
        folder,
    })
}

/// The run of words after the last `-` of the filename half, or None when
/// there is no second half (`Clannad Movie`) or nothing follows the dash.
fn movie_tail(tokens: &[String], seam: Option<usize>) -> Option<String> {
    let file = seam.map_or(0, |seam| seam + 1);
    let half = &tokens[file..];
    let dash = half.iter().rposition(|t| t == "-")?;
    let after = &half[dash + 1..];
    if after.is_empty() || dash == 0 {
        return None;
    }
    let tail = join_title(&strip_season_words(after));
    if tail.is_empty() {
        return None;
    }
    Some(tail)
}

/// Where the folder/filename seam sits in the token stream, if any.
fn seam_position(tokens: &[String]) -> Option<usize> {
    tokens.iter().position(|t| t == SEAM)
}

/// The nearest ancestor folder carrying a year, parent first. A release that
/// omits the year in its basename inherits the show folder's year.
fn ancestor_year(raw_path: &str) -> Option<u32> {
    raw_path
        .rsplit(['\\', '/'])
        .skip(1)
        .find_map(|folder| year_of(folder))
}

/// A 4-digit year (1920–2100) in the filename, e.g. `(2007)`, `.2026.`. The
/// digits must be their whole alphanumeric token, so `1920x1080` is not a
/// year.
fn year_of(name: &str) -> Option<u32> {
    fold(name)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() == 4 && token.bytes().all(|b| b.is_ascii_digit()))
        .find_map(|token| token.parse::<u32>().ok())
        .filter(|year| is_release_year(*year))
}

/// A four-digit number in the plausible release-year range (1920–2100).
fn is_release_year(number: u32) -> bool {
    (1920..=2100).contains(&number)
}

/// Drops a leftover `s2` / `2nd` / `season 2` still inside the title: the
/// marker was read, but `Grand Blue S2` keeps its `S2` in the folder and in
/// the filename, and only one form belongs in the query.
fn strip_season_words(tokens: &[String]) -> Vec<String> {
    let mut kept = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i].as_str();
        let next = tokens.get(i + 1).map(String::as_str);
        if token == "season" && next.is_some_and(|t| season_number(t).is_some()) {
            i += 2;
            continue;
        }
        // `2nd season`: the marker leads the word.
        if next == Some("season") {
            if let Some((_, rest)) = split_ordinal(token) {
                if rest.is_empty() {
                    i += 2;
                    continue;
                }
            }
        }
        // `Show II - 01`: a roman season marker already read.
        if roman(token).is_some()
            && (i + 1 == tokens.len() || next == Some("-") || next == Some(SEAM))
        {
            i += 1;
            continue;
        }
        // `s2`, `s02`: only a whole token, so `sama` stays.
        if token.len() > 1
            && token.starts_with('s')
            && token[1..].chars().all(|c| c.is_ascii_digit())
            && season_number(&token[1..]).is_some()
        {
            i += 1;
            continue;
        }
        if let Some((_, rest)) = split_ordinal(token) {
            if rest.is_empty() {
                i += 1;
                continue;
            }
        }
        kept.push(token.to_owned());
        i += 1;
    }
    kept
}

/// Whether `path` lives inside any of the Roots, compared component by
/// component and case-insensitively: `D:\Anime2` is not inside `D:\Anime`.
/// The parser never runs for a path outside every Root.
pub fn within_roots(path: &str, roots: &[String]) -> bool {
    if roots.is_empty() {
        return false;
    }
    let Some(file) = components(path) else {
        return false;
    };
    roots
        .iter()
        .filter_map(|root| components(root))
        .any(|root| file.len() > root.len() && file[..root.len()] == root[..])
}

fn is_local(path: &str) -> bool {
    let path = path.trim();
    !path.is_empty()
        && !path.contains("://")
        && !path.starts_with(r"\\")
        && !path.to_ascii_lowercase().starts_with("file:")
}

/// Canonical path components, lowercased: `c:`, `!anime`, …, the file.
fn components(path: &str) -> Option<Vec<String>> {
    use std::path::{Component, Path};
    if !is_local(path) {
        return None;
    }
    let mut parts = Vec::new();
    for component in Path::new(path.trim()).components() {
        match component {
            Component::Prefix(prefix) => parts.push(
                prefix
                    .as_os_str()
                    .to_string_lossy()
                    .trim_end_matches('\\')
                    .to_owned(),
            ),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => return None,
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
        }
    }
    let lower: Vec<String> = parts.iter().map(|p| p.to_lowercase()).collect();
    (!lower.is_empty()).then_some(lower)
}

fn parent(raw_name: &str) -> Option<String> {
    let folder = raw_name.rsplit(['\\', '/']).nth(1)?;
    (!folder.is_empty()).then(|| folder.to_owned())
}

/// The file name without its `.<ext>`.
fn stem(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && ext.len() <= 5 => {
            stem.to_owned()
        }
        _ => name.to_owned(),
    }
}

#[derive(Debug)]
struct Token {
    text: String,
    /// Stood in `[...]`: `[12]` beside an absolute episode number, or a tag
    /// glued to a word (`[Judas]No Game`).
    bracketed: bool,
}

/// Removes what a release group adds: `[…]` tags, `(…)` technical phrases and
/// years, checksums, codec/source words and a trailing group name. A `[`/`]`
/// pair must balance, or the file is rejected.
fn clean_tokens(name: &str) -> Result<Vec<Token>, Rejected> {
    // `_` is a word separator (`Guimi_Zhi_Zhu`); `-` is kept as its own token
    // so `Title - 01` is distinguishable from `Edgerunners 2` and `season 2`.
    let folded = fold(name).replace('_', " ");
    // Split a leading `bit-depth` claim (`10 bits`, `10.bits`) off before the
    // main scan: it is a tag, and its bare number would look like an episode.
    let folded = strip_bit_depths(&folded.replace('.', " "));
    let mut out: Vec<Token> = Vec::new();
    // The plain `…` word being read, and the words inside `(…)`.
    let mut word = String::new();
    let mut paren: Option<Vec<String>> = None;
    let mut chars = folded.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '[' => {
                if paren.is_some() {
                    continue;
                }
                push_word(&mut word, &mut out);
                let mut tag = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == ']' {
                        closed = true;
                        break;
                    }
                    tag.push(c);
                }
                if !closed {
                    return Err(Rejected::Unsupported("unbalanced bracket"));
                }
                // A tag contributes only an absolute-episode number (`[12]`).
                if let Some(number) = tag
                    .split_whitespace()
                    .find(|t| t.len() <= 3 && t.chars().all(|c| c.is_ascii_digit()))
                {
                    out.push(Token {
                        text: number.to_owned(),
                        bracketed: true,
                    });
                }
            }
            ']' => return Err(Rejected::Unsupported("unbalanced bracket")),
            '(' => {
                push_word(&mut word, &mut out);
                paren = Some(Vec::new());
            }
            ')' => {
                if let Some(mut inner) = paren.take() {
                    // The last word before `)` is still pending; flush it so
                    // `(2007)` is judged as a whole phrase, not leaked.
                    if let Some(w) = take(&mut word) {
                        inner.push(w);
                    }
                    // `(Ordinal Scale)` is a subtitle; `(10bit 1080p x265)`,
                    // `(Dual Audio, Multi Subs)` and `(2007)` are tags.
                    let tag_phrase = inner
                        .iter()
                        .all(|w| is_year(w) || is_metadata_word(w) || is_tech_tag(w));
                    if !tag_phrase {
                        for w in inner.join(" ").split_whitespace() {
                            push_text(w, &mut out);
                        }
                    }
                }
            }
            _ => {
                let separator = c.is_whitespace() || c == '.' || c == ',';
                if let Some(inner) = paren.as_mut() {
                    if separator {
                        if let Some(w) = take(&mut word) {
                            inner.push(w);
                        }
                    } else if c.is_alphanumeric() {
                        word.push(c);
                    }
                } else if separator {
                    push_word(&mut word, &mut out);
                } else if c == '-' {
                    // `Title - 01`: a real separator, kept as its own token.
                    push_word(&mut word, &mut out);
                    if out.last().is_none_or(|t| t.text != "-") {
                        out.push(Token {
                            text: "-".to_owned(),
                            bracketed: false,
                        });
                    }
                } else if c.is_alphanumeric() || c == '\'' {
                    word.push(c);
                } else if let Some(w) = take(&mut word) {
                    // `!`, `"`, `+` end a word; they are never query tokens.
                    push_text(&w, &mut out);
                }
            }
        }
    }
    if paren.is_some() {
        return Err(Rejected::Unsupported("unbalanced bracket"));
    }
    push_word(&mut word, &mut out);
    // `-VARYG`, `-EMBER`, `-ToonsHub`: a group tails the last title word.
    while out.len() > 1 && is_release_group(out.last().map(|t| t.text.as_str()).unwrap_or("")) {
        out.pop();
    }
    Ok(out)
}

fn take(text: &mut String) -> Option<String> {
    (!text.is_empty()).then(|| std::mem::take(text))
}

/// `10 bits`, `8 bit`: a bare bit depth, which would otherwise read as an
/// episode number once `bits` is dropped as a tag word.
fn strip_bit_depths(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').collect();
    let mut kept: Vec<&str> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let depth = i + 1 < words.len()
            && words[i].len() <= 2
            && words[i].chars().all(|c| c.is_ascii_digit())
            && matches!(words[i + 1], "bits" | "bit");
        if depth {
            i += 2;
            continue;
        }
        kept.push(words[i]);
        i += 1;
    }
    kept.join(" ")
}

/// Ends the current word: metadata, checksums and bare numbers are dropped.
fn push_word(word: &mut String, out: &mut Vec<Token>) {
    if let Some(word) = take(word) {
        push_text(&word, out);
    }
}

fn push_text(word: &str, out: &mut Vec<Token>) {
    let text = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
    if text.is_empty() || is_metadata_word(text) || is_tech_tag(text) || is_checksum(text) {
        return;
    }
    let text = expand_ordinal(text);
    // A bit depth shown as its own number (`1080p BDRip 10 bits`): the word
    // after it was a tag, so the number would be mistaken for an episode.
    if is_number(&text) && text.len() <= 2 {
        if let Some(Token { text: previous, .. }) = out.last() {
            if previous.ends_with('p') || previous.ends_with("bits") {
                return;
            }
        }
    }
    out.push(Token {
        text,
        bracketed: false,
    });
}

/// Known release groups that survive a bracket strip as a trailing word.
fn is_release_group(word: &str) -> bool {
    const KNOWN: &[&str] = &[
        "judas",
        "asw",
        "trix",
        "db",
        "cleo",
        "bd",
        "bdfansub",
        "sokudo",
        "erai",
        "raws",
        "ember",
        "toonshub",
        "varyg",
        "horriblesubs",
        "subsplease",
        "gg",
        "neko",
        "kametsu",
        "yunsha",
        "nii",
        "sama",
        "lilith",
        "pid",
        "fff",
        "zura",
        "shiratsu",
    ];
    KNOWN.contains(&word)
}

/// A token that only marks a file as an episode (`S02E05`, `01`, `episode 5`).
fn is_episode_metadata(token: &str) -> bool {
    token == "-"
        || is_number(token)
        || sxxexx(token).is_some()
        || marker_leading(std::slice::from_ref(&token.to_owned())).is_some()
        || token == "season"
        || ["ep", "episode"]
            .iter()
            .any(|p| token.strip_prefix(p).is_some_and(is_number))
}

/// Splits a season marker off either half of the token stream. `:` is the
/// folder/filename seam; the marker may lead or trail a title
/// (`Grand Blue S2`, `S2 - 01`, `2nd Season - 01`, `… - 01 S2`).
fn split_season(tokens: Vec<String>) -> (Vec<String>, Option<u32>) {
    let Some(seam) = tokens.iter().position(|t| t == SEAM) else {
        return (tokens, None);
    };
    let folder = &tokens[..seam];
    let file = &tokens[seam + 1..];

    // Filename: `S2 - 01`, `2nd Season - 01`, `… - 01 S2`, `Show S3 - 01`.
    let season = marker_leading(file).or_else(|| marker_ending(file).map(|(n, _)| n));
    let season = season.or_else(|| marker_in(file));
    // Folder: `Honzuki no Gekokujou S2`, `Vinland Saga Season 2`.
    let season = season
        .or_else(|| marker_ending(folder).map(|(n, _)| n))
        .or_else(|| marker_in(folder));
    // The tokens stay as read: `strip_season_words` removes the marker from
    // whichever half it sits in.
    (tokens, season)
}

/// `season 2 …`, `s2 …`, `2nd …` at the head of a half.
fn marker_leading(tokens: &[String]) -> Option<u32> {
    match tokens {
        [head, number, ..] if head == "season" => season_number(number),
        [head, ..] => marker_token(head),
        _ => None,
    }
}

/// Folder: `Honzuki no Gekokujou S2`, `Vinland Saga Season 2`, `Oshi no Ko
/// S3`. A marker anywhere in the folder is the season, since the folder is a
/// show name and carries no episode numbers.
fn marker_in(tokens: &[String]) -> Option<u32> {
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "season" {
            if let Some(n) = tokens.get(i + 1).and_then(|t| season_number(t)) {
                return Some(n);
            }
            i += 1;
            continue;
        }
        if let Some(n) = marker_token(&tokens[i]) {
            return Some(n);
        }
        i += 1;
    }
    None
}

/// `s2`, `2nd`, `ii` after a movie marker has been removed: `S3 - 01`.
fn marker_ending(tokens: &[String]) -> Option<(u32, usize)> {
    let last = tokens.len();
    let tail = |n: usize| {
        last.checked_sub(n)
            .and_then(|i| tokens.get(i))
            .map(String::as_str)
    };

    if tail(2) == Some("season") {
        return tail(1).and_then(season_number).map(|n| (n, last - 2));
    }
    let token = tail(1)?;
    // `s2`, `s02`: a whole token, never a word that merely starts with `s`.
    if token.len() > 1
        && token.starts_with('s')
        && token[1..].chars().all(|c| c.is_ascii_digit())
        && season_number(&token[1..]).is_some()
    {
        return Some((season_number(&token[1..])?, last - 1));
    }
    if last >= 2 {
        // `… ii` / `… iii` after a title word; never a lone numeral.
        return tail(1).and_then(roman).map(|n| (n, last - 1));
    }
    None
}

/// One token as a season marker: `S2`, `s02`, `2nd`, `season 2`, `II`.
fn marker_token(token: &str) -> Option<u32> {
    // `s02e05` is an episode marker, not a season one.
    if sxxexx(token).is_some() {
        return None;
    }
    if let Some(rest) = token.strip_prefix('s') {
        if rest.chars().all(|c| c.is_ascii_digit()) {
            return season_number(rest);
        }
    }
    match split_ordinal(token) {
        Some((n, "")) => Some(n),
        Some(_) => None,
        None => roman(token),
    }
}

/// `2`, `02`, `2nd`, `ii` → 2. Only 1–50 are seasons.
fn season_number(token: &str) -> Option<u32> {
    if let Ok(n) = token.parse::<u32>() {
        return (1..=50).contains(&n).then_some(n);
    }
    if let Some((n, "")) = split_ordinal(token) {
        return Some(n);
    }
    roman(token)
}

/// `2nd` → (2, ""), `2nd-season` → (2, "season"), `1st` → (1, "").
fn split_ordinal(token: &str) -> Option<(u32, &str)> {
    let digits = token.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let n = token[..digits].parse::<u32>().ok()?;
    let rest = &token[digits..];
    let (suffix, after) = (rest.get(..2)?, rest.get(2..)?);
    let last_two = n % 100;
    let matches = match suffix {
        "st" => n % 10 == 1 && last_two != 11,
        "nd" => n % 10 == 2 && last_two != 12,
        "rd" => n % 10 == 3 && last_two != 13,
        "th" => {
            !(n % 10 == 1 && last_two != 11)
                && !(n % 10 == 2 && last_two != 12)
                && !(n % 10 == 3 && last_two != 13)
        }
        _ => false,
    };
    if !matches {
        return None;
    }
    if after.is_empty() {
        return Some((n, ""));
    }
    if after.starts_with(|c: char| !c.is_alphanumeric()) {
        return Some((n, after.trim_start_matches(|c: char| !c.is_alphanumeric())));
    }
    // `2nd-seasons` arrives as `2nd` + `seasons`: not a marker.
    Some((n, "other"))
}

/// `iv`, `vii`, `xii` → 4, 7, 12. Only 2–19 are season markers: a lone
/// `i`/`x` is a title letter (`Infinity Castle I`, `To Be Hero X`), and a
/// lone `I` is also season 1 in `Show I`.
fn roman(token: &str) -> Option<u32> {
    if token.len() < 2 || token.len() > 4 || !token.chars().all(|c| "ivx".contains(c)) {
        return None;
    }
    let mut total = 0;
    let mut previous = 0;
    for c in token.chars().rev() {
        let value = match c {
            'i' => 1,
            'v' => 5,
            'x' => 10,
            _ => return None,
        };
        if value < previous {
            total -= value;
        } else {
            total += value;
            previous = value;
        }
    }
    (1..=19).contains(&total).then_some(total)
}

/// `Movie 01 …`, `… - Movie 01 …`: the marker and its number are removed. A
/// bare `movie` word is part of a title (`Clannad Movie`, `Ansatsu
/// Kyoushitsu Movie - Minna no Jikan`) and stays.
fn strip_movie_marker(tokens: Vec<String>) -> (Vec<String>, bool) {
    let Some(pos) = tokens.iter().position(|t| t == "movie") else {
        return (tokens, false);
    };
    let numbered = tokens.get(pos + 1).is_some_and(|t| is_number(t));
    if !numbered {
        return (tokens, false);
    }
    let mut kept = tokens;
    kept.drain(pos..pos + 2);
    (kept, true)
}

/// Kept in step with `strip_bit_depths`: bit-depth numbers are already gone,
/// so only a codec's channel layout (`DDP5 1`) remains as a tag number.
fn is_tag_number(tokens: &[String], index: usize) -> bool {
    index
        .checked_sub(1)
        .and_then(|i| tokens.get(i))
        .is_some_and(|t| is_channel_layout(t))
}

/// Cuts the episode marker out: `S02E01` anywhere in the filename (its title
/// follows, e.g. `S01E01-A World Without Books`), or at the end `- 01`, `001`,
/// `E05`, `EP05`, `Episode 05`. A folder's numbers are never episodes.
/// A token that marks a batch or multi-episode file: `01-12` (read as
/// `01 12`), `01&02`, `01+02`. A version suffix (`01v2`) is not one: it
/// re-releases the same episode, and only the number counts.
fn is_range_token(token: &str) -> bool {
    let parts: Vec<&str> = token
        .split(['&', '+', '~', '/'])
        .filter(|p| !p.is_empty())
        .collect();
    parts.len() > 1 && parts.iter().all(|p| is_episode_number(p))
}

/// A flattened range (`01 12`, `01 02`) is in the filename, not the folder.
/// Metadata numbers (`10 bits`, `H.264`, `season 2`) never form one.
fn split_episode(tokens: Vec<String>) -> Result<(Vec<String>, Option<u32>), Rejected> {
    let file = tokens
        .iter()
        .position(|t| t == SEAM)
        .map_or(0, |seam| seam + 1);
    let half = &tokens[file..];
    if half.iter().any(|t| is_range_token(t)) {
        return Err(Rejected::Unsupported("batch range or multi-episode file"));
    }
    for (idx, token) in half.iter().enumerate() {
        if let Some((_, n)) = sxxexx(token) {
            return Ok((tokens[..file + idx].to_vec(), Some(n)));
        }
    }

    // Episode words and numbers only: `Show Season 2 - 01` ends in `01`, and
    // `… 10 bits` ends in a bit depth. A versioned episode (`01v2`) counts as
    // its base number.
    let numbers: Vec<(usize, u32)> = half
        .iter()
        .enumerate()
        .filter(|&(i, _)| !is_tag_number(half, i))
        .filter_map(|(i, t)| {
            let number = versioned_episode(t).unwrap_or(t);
            is_episode_number(number).then(|| (i, number.parse::<u32>().unwrap_or(0)))
        })
        .collect();
    // Two numbers in a row at the end (`Show - 01 12`): a flattened range.
    if numbers.len() >= 2 {
        let (first, second) = (numbers[numbers.len() - 2], numbers[numbers.len() - 1]);
        let adjacent = second.0 == first.0 + 1 || second.0 == first.0 + 2;
        let end = second.0 + 1 == half.len();
        // `01 12`, `01 02`: both look like episodes. `2026 1 264` ends in a
        // single digit and a codec remnant, not a range.
        if adjacent && end && half[first.0].len() >= 2 && half[second.0].len() <= 2 {
            return Err(Rejected::Unsupported("batch range or multi-episode file"));
        }
    }

    // `Title - 01` is an episode; `Edgerunners 2` or `Season 2` is a title
    // word or marker. A bare number needs a `-` (or `ep`/`episode`) before it,
    // or a filename that is nothing but markers (`S02E05` handling above).
    // `Edgerunners 2 - 01` and `Season 2 - 01` have a real marker; a year
    // before the `-` (`Kaguya 2026 - 1 264`) is a split tag.
    let marked = |idx: usize| {
        idx > 0
            && matches!(half[idx - 1].as_str(), "-" | "ep" | "episode")
            && !(idx >= 2 && is_year(half[idx - 2].as_str()))
    };
    // A number with no marker is only an episode when it is all the filename
    // has after the title (`01.mkv`, `S02E05`): never a year's neighbour.
    let lone = |idx: usize| idx == 0 && half.len() == 1;
    let Some(&(idx, n)) = numbers
        .iter()
        .rev()
        .find(|(idx, _)| marked(*idx) || lone(*idx))
    else {
        return Ok((tokens, None));
    };
    // Nothing but markers may follow the episode number.
    let trailing = half[idx + 1..]
        .iter()
        .all(|t| t == "-" || is_episode_metadata(t));
    if !trailing {
        return Ok((tokens, None));
    }
    let start = file + idx - usize::from(marked(idx));
    let mut kept = tokens;
    kept.truncate(start);
    if kept.last().is_some_and(|t| t == "-") {
        kept.pop();
    }
    Ok((kept, Some(n)))
}

/// `01v2`, `1071v02` → `01`, `1071`: a version suffix marks a re-release of
/// the same episode. Only a one- or two-digit version counts, and the base
/// must itself be an episode number.
fn versioned_episode(token: &str) -> Option<&str> {
    let (number, version) = token.split_once('v')?;
    (!version.is_empty()
        && version.len() <= 2
        && version.bytes().all(|b| b.is_ascii_digit())
        && is_episode_number(number))
    .then_some(number)
}

/// `01`, `001`, `13`, `1071`: an episode-sized number. A four-digit number in
/// the year range is a year, never an episode (`Show - 2026`); `ONE PIECE -
/// 1071` is an episode.
fn is_episode_number(token: &str) -> bool {
    if !is_number(token) || token.len() > 4 {
        return false;
    }
    let Ok(number) = token.parse::<u32>() else {
        return false;
    };
    number > 0 && !(token.len() == 4 && is_release_year(number))
}

fn is_number(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit())
}

/// `s02e01` → (2, 1); `s01ova` and `show` are not one.
fn sxxexx(token: &str) -> Option<(u32, u32)> {
    let digits = token
        .strip_prefix('s')?
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    if digits == 0 {
        return None;
    }
    let season = token[1..1 + digits].parse::<u32>().ok()?;
    let rest = token[1 + digits..].strip_prefix('e')?;
    Some((season, rest.parse::<u32>().ok()?))
}

fn is_year(token: &str) -> bool {
    token.len() == 4 && token.starts_with(|c: char| c.is_ascii_digit())
}

fn join_title(tokens: &[String]) -> String {
    tokens
        .iter()
        .filter(|t| t.as_str() != "-")
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric() && c != '\''))
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The query form: `Season N` for N > 1, plain title otherwise (`S1` and
/// `Season 1` both mean season 1).
fn with_season(title: String, season: Option<u32>) -> String {
    match season {
        Some(n) if n > 1 => format!("{title} season {n}"),
        _ => title,
    }
}

/// Resolution, codec, source, audio and subtitle words: release tags, never
/// title words. A title word that happens to match keeps its place only when
/// it is part of the name (`No Game No Life Zero`).
fn is_metadata_word(token: &str) -> bool {
    const KNOWN: &[&str] = &[
        "1080p",
        "720p",
        "2160p",
        "480p",
        "4k",
        "8k",
        "10bit",
        "8bit",
        "12bit",
        "10bits",
        "bits",
        "bit",
        "hi10p",
        "h264",
        "h265",
        "x264",
        "x265",
        "hevc",
        "avc",
        "av1",
        "aac",
        "aac2",
        "ac3",
        "eac3",
        "flac",
        "dts",
        "opus",
        "mp3",
        "ddp",
        "dd",
        "truehd",
        "bd",
        "bdrip",
        "brrip",
        "bdremux",
        "remux",
        "web",
        "webdl",
        "webrip",
        "dl",
        "hdtv",
        "hulu",
        "nf",
        "cr",
        "adn",
        "amzn",
        "jpn",
        "eng",
        "esp",
        "ita",
        "ger",
        "fra",
        "jap",
        "dual",
        "multi",
        "multisub",
        "msubs",
        "msub",
        "audio",
        "subs",
        "esub",
        "sub",
        "subbed",
        "uncensored",
        "repack",
        "uncut",
        "v2",
        "v3",
        "h",
        "rip",
        "264",
        "265",
    ];
    KNOWN.contains(&token) || is_channel_layout(token)
}

/// `ddp5`, `aac2`, `5`, `1`: a codec glued to its channel layout, already
/// split by the `.` rule (`DDP5.1` → `ddp5` `1`).
fn is_channel_layout(token: &str) -> bool {
    const CODECS: &[&str] = &[
        "ddp", "dd", "aac", "ac3", "eac3", "dts", "flac", "opus", "truehd",
    ];
    CODECS.iter().any(|codec| {
        token
            .strip_prefix(codec)
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// A word that only ever appears in a technical tag: a `(…)`/`[…]` phrase
/// containing one is a tag phrase, not a subtitle.
fn is_tech_tag(token: &str) -> bool {
    const KNOWN: &[&str] = &[
        "webrip",
        "webdl",
        "web",
        "bd",
        "bdrip",
        "hdtv",
        "hevc",
        "av1",
        "x265",
        "x264",
        "1080p",
        "720p",
        "2160p",
        "480p",
        "4k",
        "8k",
        "10bit",
        "8bit",
        "12bit",
        "dual",
        "multi",
        "msub",
        "msubs",
        "audio",
        "subs",
        "eac3",
        "aac",
        "flac",
        "dts",
        "hi10p",
        "uncensored",
        "repack",
        "remux",
        "avc",
        "opus",
        "truehd",
    ];
    KNOWN.contains(&token)
        || (token.starts_with("bd") && token.ends_with('p'))
        || (token.starts_with("dvd") && token.ends_with('p'))
}

/// An upper-case hex or all-digit tag of eight or more characters: a CRC32 or
/// a fansub id (`49889AC3`, `D4935429`, `2E01273A`).
fn is_checksum(token: &str) -> bool {
    token.len() >= 8 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The season marker a title carries, if any: `2nd Season`, `Season 2`, `II`,
/// `3rd Season`. Matching uses it to corroborate an explicit season marker in
/// the Release Filename (`Honzuki S4` passes through the synonym `4th
/// Season`; `Youjo Senki II` through the roman numeral).
pub fn title_season(text: &str) -> Option<u32> {
    let tokens: Vec<String> = fold_words(text).split(' ').map(str::to_owned).collect();
    marker_in(&tokens)
}

/// The comparison form for titles and queries: lowercased, accents folded,
/// punctuation collapsed to single spaces (`Sword x Family!` → `sword x family`).
pub fn fold_words(text: &str) -> String {
    fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lowercase with accents folded, so `Pokémon` and `Pokemon` agree.
fn fold(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'À'..='Å' | 'à'..='å' => 'a',
            'Æ' | 'æ' => 'e',
            'Ç' | 'ç' => 'c',
            'È'..='Ë' | 'è'..='ë' => 'e',
            'Ì'..='Ï' | 'ì'..='ï' => 'i',
            'Ð' | 'ð' => 'd',
            'Ñ' | 'ñ' => 'n',
            'Ò'..='Ö' | 'ò'..='ö' | 'Ø' | 'ø' => 'o',
            'Ù'..='Ü' | 'ù' | 'ü' => 'u',
            'Ý' | 'ý' | 'ÿ' => 'y',
            'Þ' | 'þ' => 't',
            'ß' => 's',
            'Œ' | 'œ' => 'e',
            other => other,
        })
        .collect::<String>()
        .to_lowercase()
}

/// `second` → `2nd`, so one season-marker parser sees `2nd`, `Season 2` and
/// `S2` alike.
fn expand_ordinal(word: &str) -> String {
    const ORDINALS: &[(&str, u32)] = &[
        ("first", 1),
        ("second", 2),
        ("third", 3),
        ("fourth", 4),
        ("fifth", 5),
        ("sixth", 6),
        ("seventh", 7),
        ("eighth", 8),
        ("ninth", 9),
        ("tenth", 10),
    ];
    match ORDINALS.iter().find(|(w, _)| *w == word) {
        Some((_, n)) => ordinal(*n),
        None => word.to_owned(),
    }
}

/// `2` → `2nd`, `3` → `3rd`, `11` → `11th`.
fn ordinal(n: u32) -> String {
    let last_two = n % 100;
    let suffix = if matches!(last_two, 11..=13) {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{n}{suffix}")
}
