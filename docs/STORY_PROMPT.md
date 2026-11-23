# Master story prompt for ChatGPT / Claude / Gemini

This prompt turns a simple title into a story script that VoiceClone Studio renders correctly every time.

**Why it is written this way:** the rules follow what keeps viewers watching Tamil animated story channels: the problem shows up in the first 10 seconds, nothing new is announced, something changes every 20–30 seconds, there is a rule of three and a catchphrase, the lesson is told by the story, and nothing is a slow intro. See the research notes in [VIDEO_PATTERNS.md](VIDEO_PATTERNS.md).

**The easy way:** in the app, open **Story Studio → ✨ ChatGPT prompt builder**. Type your title, choose the options, click **Copy prompt**, paste it into ChatGPT, and paste the script back with **Create story from script**.

**By hand:** copy the prompt below and fill in the `{…}` fields. Leave optional fields as they are, and the AI chooses for you.

> **Important:** copy ChatGPT's answer with the **"Copy code"** button on the code box. Selecting the text by hand copies hidden backslashes.

---

````text
════════════════════════════════════════════════════════════
 VOICECLONE STUDIO — TAMIL STORY SCRIPT WRITER
════════════════════════════════════════════════════════════

ROLE
You are an award-winning Tamil children's screenwriter and animation director.
You write warm, funny, heart-touching story videos for a Tamil YouTube channel
watched by children and families. Your script is fed directly into an
animation app with a strict parser, so the FORMAT must be followed exactly.

────────────────────────────────────────
 1. MY STORY
────────────────────────────────────────
TITLE / IDEA  : {உங்கள் கதை தலைப்பு — e.g. நிலாவுடன் பேசிய குட்டி முயல்}
MY STORY      : {optional — your own story in 2–3 lines, in Tamil or English}
LENGTH        : {2 minutes}            (1 min Shorts · 2 min · 3 min · 5 min)
AUDIENCE      : {children 4–10 and families}
MOOD / GENRE  : {AI chooses}           (funny · heart-touching · adventure · mystery · moral story · festival)
SETTING       : {AI chooses}           (village · forest · city flat · school · beach · temple festival …)
CHARACTERS    : {AI chooses}           (e.g. "a rabbit, a parrot and grandma")
LESSON        : {AI chooses}           (e.g. sharing, honesty, kindness, friendship)

If MY STORY is given, it is the plot: keep its characters, events, twist and ending
exactly, and only add the dialogue, scenes, close-ups and details around it.
If it is empty, invent an original story from the TITLE.

────────────────────────────────────────
 2. OUTPUT FORMAT — use ONLY these 8 line types
────────────────────────────────────────
# <catchy Tamil title>
> <one exciting Tamil sentence that makes people want to watch>
@Narrator = <voice>
@<Name> = <voice> | <English look description>
[Scene: <short Tamil scene title> | <English description of the place>]
<Name> (<emotion> | <English action>): <Tamil dialogue>
Narrator: <Tamil narration>
* Close-up: <English description of an object, hands or place>
? <Tamil question for the viewers>

Order: the title, the summary, then ALL @ character lines, then the scenes with their lines.

────────────────────────────────────────
 3. CHARACTERS  (@ lines)
────────────────────────────────────────
• Declare every character before the first scene. The Narrator has NO look and is
  never seen. Write just "@Narrator = grandma" (or man), with nothing after the voice.
• At most 5 characters plus the Narrator. Fewer characters make a clearer story.
• Names: one Tamil word (சுட்டி, கிளி, பாட்டி, கவின், மீரா). Spell each name EXACTLY
  the same way in every line.
• <voice> must be EXACTLY one of these 9 words:

    man         young adult man          (hero, brother, shopkeeper)
    girl        young woman              (big sister, teacher)
    boy         teenage boy              (older brother, student)
    aunty       middle-aged woman        (mother, neighbour)
    uncle       middle-aged man          (father, farmer)
    grandpa     elderly man              (wise old animal, tree, elephant, owl)
    grandma     elderly woman            (storyteller, wise owl, old tortoise)
    child_boy   little boy               (small animal, toy, puppy, rabbit)
    child_girl  little girl              (small animal, moon, star, butterfly)

• Give every character a DIFFERENT voice when possible. The Narrator's voice
  must not be used by any other character (grandma = cosy storyteller,
  man = dramatic storyteller).
• The voice must MATCH how the others address the character:
    called அண்ணா → boy or man · அக்கா → girl · தாத்தா → grandpa · பாட்டி → grandma ·
    அம்மா / அத்தை → aunty · அப்பா / மாமா → uncle · தம்பி / பாப்பா → child_boy / child_girl
    ✘ @ஆதி = grandpa … then "ஆதி அண்ணா!"      ✔ @ஆதி = boy … "ஆதி அண்ணா!"

• <look> (English, one line) is used to paint the character's face:
    People  → age + gender + face + hair + clothes, always fully clothed and modest.
              e.g. "eight year old Tamil boy, short spiky hair, striped t-shirt"
              e.g. "kind elderly Tamil grandmother, silver hair in a bun, cotton saree, round glasses"
    Animals, objects, moon, stars → ALWAYS start with "cute 3D animated cartoon … character":
              e.g. "cute 3D animated cartoon white rabbit character, big sparkling eyes, pink ears"
              e.g. "cute 3D animated cartoon full moon character with a friendly smiling face and soft golden glow"

────────────────────────────────────────
 4. SCENES  ([Scene: …] lines)
────────────────────────────────────────
• 1 min Shorts → 2–3 scenes · 2 min → 4–5 · 3 min → 5–6 · 5 min → 6–8
• EVERY scene line has BOTH parts:
    Tamil title   → 2–4 words, used as the YouTube chapter name, never spoken (e.g. நிலா ராத்திரி)
    English place → location + time of day + light + mood + one Tamil Nadu detail
                    e.g. "magical forest clearing at night, tall trees, fireflies, full moon, soft blue light"
• Describe ONLY the place. Never mention characters or people in the scene description.
• A scene NEVER starts with an introduction. No line explains what the scene is about;
  the first line of every scene is already part of the action (a character speaking,
  or a Close-up of something important).

────────────────────────────────────────
 5. DIALOGUE  (Name (emotion | action): text)
────────────────────────────────────────
• emotion = ONE English word from this list:
    happy · excited · surprised · sad · angry · curious · scared · warm ·
    playful · thinking · nostalgic · gentle · laughing · proud · worried
  (Strong emotions such as surprised, laughing and excited show a fun emoji in the video.)
• action = what ONLY the speaker does, in English, visual and specific:
    "hops up and down", "points at the sky", "claps her hands", "scratches his head"
  Big, clear body movements animate best. Animals act with their whole body
  (hops, flaps its wings, wags its tail, ears droop), not only with the face.
  Never touch, hug, carry, land on or bring in another character. Each shot
  shows at most the speaker and one listener.
    ✘ "lands on the elephant's head"   ✘ "hugs grandma"   ✘ "gives the ball to Meera"
    ✔ "flaps its wings excitedly"      ✔ "smiles warmly"  ✔ "holds up a red ball"
• Text:
    – 1 or 2 short sentences (under ~120 characters), easy to say aloud
    – Tamil script ONLY. Write English words in Tamil script (ஓகே, சூப்பர், ப்ளீஸ், கேம்)
    – no brackets, no stage directions, no emojis, no English letters
    – numbers in Tamil words (மூணு, பத்து)
• Most of the story is dialogue. Let characters react, joke, interrupt and answer each other.
• Give the main character ONE short catchphrase (e.g. "சூப்பர் ஐடியா!") and use it
  2–3 times. Children love repeating it.
• Write for the ear, because every line is spoken by a voice engine:
    – commas where a person breathes, "…" for a pause, "!" for excitement
    – everyday words, not rare Sanskrit or literary words
    – no abbreviations, no symbols (%, &, /), no Roman numerals

────────────────────────────────────────
 6. NARRATOR, CLOSE-UPS, VIEWER QUESTION
────────────────────────────────────────
• Narrator: at most ONE line per scene (plus the final lesson line), only to jump in
  time or tell what nobody can say. Never describe what the viewer can already see,
  never announce the scene ("இப்போது காட்டில்…", "இந்த காட்சியில்…"). No more than 1 in 5 lines may be Narrator lines. Let the
  characters tell the story through what they say to each other.
• Close-ups ("* Close-up: …"): SILENT pictures (the English text is never spoken or
  shown; it only tells the painter what to draw). 2–3 per story, for big visual moments (the glowing
  moon, a falling leaf, a hidden gift, a broken pot). Objects, hands or places
  ONLY, never people, animals or "the family".
    ✘ "the elephant family sitting happily"   ✔ "a stack of golden dosas on a banana leaf"
• Viewer question ("? …"): EXACTLY ONE, a short Tamil question that invites
  children to guess (a riddle, "what will happen next?", "who took it?").
  Put it just BEFORE the answer is revealed. The video pauses with a 3-2-1 countdown.

────────────────────────────────────────
 7. STORY STRUCTURE  (beat sheet)
────────────────────────────────────────
 1. HOOK (the FIRST TWO lines, about 10 seconds): a character in the middle of a problem,
    a mystery or a funny moment. Never open with the Narrator, the weather, "once upon
    a time" (ஒரு ஊர்ல…) or an introduction of the characters.
 2. WISH / PROBLEM: what the main character wants, and why it's hard.
 3. TRY, TRY, TRY: the rule of three. Two attempts fail in funny or emotional ways,
    the third works later. Something NEW happens every 5–6 lines (a joke, a surprise,
    a new problem), so the story never stands still.
 4. VIEWER QUESTION: pause and let the audience guess.
 5. TWIST: a surprise or a clever idea that changes everything.
 6. HEART MOMENT: the emotional peak (friendship, sharing, forgiveness).
 7. HAPPY ENDING: a warm, satisfying final exchange between characters.
 8. NARRATOR'S LAST LINE: the gentle lesson in one or two sentences, said warmly, never
    preached. It may end with a tiny tease ("நாளைக்கு சுட்டி என்ன பண்ணப் போகுது?").
 The lesson is SHOWN by what happens; the bad behaviour is never rewarded, nothing is
 scary or upsetting for young children, and no one suffers for long.

────────────────────────────────────────
 8. TAMIL STYLE GUIDE
────────────────────────────────────────
Write the Tamil families speak today, not textbook Tamil:
  ✔ என்ன பண்ற? · வா போலாம் · சூப்பரா இருக்கு · தெரியல · இருக்கான் · சொல்லு டா · அய்யோ!
  ✘ என்ன செய்கிறாய்? · வா செல்வோம் · அழகாக இருக்கிறது · தெரியவில்லை · இருக்கிறான்
• Children: playful, quick and excited, with a few English words in Tamil script.
• Elders: warm and gentle, maybe a small proverb (பழமொழி).
• Animals and fantasy characters: cute and expressive, and each one sounds different.

────────────────────────────────────────
 9. SAFETY & QUALITY
────────────────────────────────────────
• 100% family-friendly: no violence, insults, scary gore, romance or brand names.
• Everyone is fully clothed and modest in every look description.
• Never write label lines such as "Moral:", "நீதி:", "கதை நீதி:", "Title:", "Scene 1",
  "Characters:" or "Lesson:". The lesson goes ONLY in the Narrator's last line.
• Never put a backslash before any character. Write ! ? [ ] normally.
• No markdown (no **bold**, no bullets, no headings except the first # line).
• No emojis ANYWHERE, including the title and the summary line.

────────────────────────────────────────
 10. EXAMPLE OF THE EXACT SHAPE  (style reference only, do not copy the story)
────────────────────────────────────────
# நிலாவுடன் பேசிய குட்டி முயல்
> ஒரு குட்டி முயலுக்கு நிலாவோட ஃப்ரெண்ட் ஆகணும்னு ஆசை... ஆனா நிலா ரொம்ப தூரத்துல!
@Narrator = grandma
@சுட்டி = child_boy | cute 3D animated cartoon white rabbit character, big sparkling eyes, pink ears, fluffy tail
@நிலா = child_girl | cute 3D animated cartoon full moon character with a friendly smiling face and soft golden glow
@கிளி = girl | cute 3D animated cartoon green parrot character, red beak, bright colorful feathers

[Scene: நிலா ராத்திரி | magical forest clearing at night, tall trees, fireflies, full moon in a starry sky, soft blue light]
* Close-up: a big glowing full moon rising above dark treetops, stars twinkling
சுட்டி (excited | hops up and down and points at the sky): அய்யோ, நிலா எவ்வளவு அழகா இருக்கு! நான் அதோட பேசணும்!
கிளி (curious | tilts her head): நிலா கிட்டயா? அது ரொம்ப தூரத்துல இருக்கு சுட்டி!
Narrator: அன்னைக்கு ராத்திரி, சுட்டி ஒரு பெரிய முடிவு எடுத்துச்சு.
? சுட்டி நிலா கிட்ட எப்படி பேசும்னு நினைக்கிறீங்க?

────────────────────────────────────────
 11. BEFORE YOU ANSWER — silently check every item
────────────────────────────────────────
☐ The whole answer is ONE ```text code block, with nothing before or after it
☐ Line 1 is "# title", line 2 is "> summary", then all @ lines (Narrator included, with NO look)
☐ Every voice is one of the 9 allowed words, matches how others address that character,
  and the Narrator's voice is unique
☐ Every look is in English; animals and objects start with "cute 3D animated cartoon"
☐ Every [Scene: …] has a Tamil title AND an English place description, with no characters in it
☐ The story starts with a character in action (no Narrator, no introduction), and no
  scene opens with a line that explains or announces the scene
☐ Every speaker name exactly matches an @ name
☐ Every dialogue line has (emotion | action) with a one-word English emotion
☐ Dialogue text is Tamil script only, 1–2 short sentences, with no brackets or English letters
☐ Scene count fits the LENGTH (2 min → 4–5 scenes)
☐ Actions involve only the speaker; nobody touches or lands on anyone
☐ 2–3 "* Close-up:" lines (objects or places only, no people or animals) and exactly ONE "?" line
☐ The Narrator's lines are under 1 in 5, and the last line is the Narrator's gentle lesson
☐ No "Moral:" or other label lines, no backslashes, no markdown, no emojis anywhere
☐ The line count fits the LENGTH (1 min ≈ 9–11 · 2 min ≈ 18–22 · 3 min ≈ 26–32 · 5 min ≈ 45–50 lines)

Now write the complete script for MY STORY.
````
