// GJS exposes a gunichar return value as a one-character string, not its
// numeric code point. Preserve it before considering numeric return values.
export function keyText(unicode, keysym = 0) {
    let text = '';
    if (typeof unicode === 'string') {
        text = unicode;
    } else if (Number.isInteger(unicode) && unicode > 0 && unicode <= 0x10ffff) {
        text = String.fromCodePoint(unicode);
    }

    // Some keyboard maps provide a keypad keysym without a Unicode value.
    if (text === '' || text === '\0') {
        if (keysym >= 0xffb0 && keysym <= 0xffb9)
            text = String(keysym - 0xffb0);
        else if (keysym === 0xffae)
            text = '.';
    }

    if ([...text].length !== 1) return '';
    const code = text.codePointAt(0);
    if (code < 0x20 || (code >= 0x7f && code <= 0x9f) ||
        (code >= 0xd800 && code <= 0xdfff)) return '';
    return text;
}
