# Story script format

A story is plain text, so it's easy to write, paste, share and keep in version control.

```text
# பாட்டியின் மந்திரப் பெட்டி
> கரண்ட் போன ஒரு மழை ராத்திரி, ஒரே ஒரு டேப்லெட், ரெண்டு குழந்தைகள்...

@Narrator = man
@பாட்டி = grandma | kind elderly Tamil grandmother, silver hair in a bun, cotton saree, round glasses
@கவின் = child_boy | eight year old Tamil boy, short spiky hair, striped t-shirt

[Scene: கரண்ட் கட்! | cozy Chennai living room on a stormy night, lightning outside, lit by an oil lamp]
* Close-up: a smartphone screen showing a one percent battery warning, then going black
கவின் (shocked | stares at his dead phone and shakes it): அய்யோ! என் ஃபோன் ஆஃப் ஆயிடுச்சு!
Narrator: மழை கொட்டுது, கரண்ட் இல்ல, வீட்டுக்குள்ள ஒரே சண்டை.
? நீங்களும் யோசிங்க! அது என்னவா இருக்கும்?
```

| Line | Meaning |
|---|---|
| `# Title` | Story title (title card, thumbnail, file name) |
| `> summary` | One-line summary (title card, description) |
| `@Name = preset \| look` | Declares a character. Presets: `man`, `girl`, `boy`, `aunty`, `uncle`, `grandpa`, `grandma`, `child_boy`, `child_girl`. The look (English) is used to paint the face. |
| `[Scene: title \| description]` | Starts a scene. The title appears as a scene card (leave it empty for no card); the English description paints the background. |
| `Name: text` | A spoken line |
| `Name (emotion): text` | With an emotion: happy, sad, excited, surprised, angry, curious, warm, nostalgic, playful, thinking… Strong emotions pop an emoji sticker. |
| `Name (emotion \| action): text` | With what the speaker does (English). This drives the animation. |
| `Narrator: text` | Narration, shown over the full scene with no face |
| `* description` | **Insert shot**: a silent animated close-up (English). Show objects, hands or places, not people. |
| `? question` | **Viewer question**: the video pauses with your Tamil question and a 3-2-1 countdown |
| plain paragraph | Read by the Narrator |

Unknown speaker names become characters automatically, with a voice guessed from the name:

- பாட்டி → grandma, தாத்தா → grandpa
- அம்மா / அத்தை → aunty, அப்பா / மாமா → uncle
- அக்கா → girl, தம்பி → child boy
- முயல் / கிளி → child boy, நிலா → child girl, யானை → grandpa

Stage directions in brackets inside the spoken text, like `(சிரிக்கிறாள்)` or `[smiles]`, are removed so they aren't read aloud. Put them in the action field instead.

A full example: [`examples/paatiyin_manthira_petti.txt`](../examples/paatiyin_manthira_petti.txt).
