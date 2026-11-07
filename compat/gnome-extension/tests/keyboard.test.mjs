import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';

const source = readFileSync(new URL('../keyboard.js', import.meta.url), 'utf8');
const {keyText} = await import(`data:text/javascript,${encodeURIComponent(source)}`);

test('GJS character strings preserve every digit, decimal point and HEX character', () => {
    for (const char of '0123456789.+-#ABCDEFabcdef')
        assert.equal(keyText(char), char);
});

test('numeric Unicode return values decode without coercing character strings', () => {
    for (const char of '0123456789.+-#ABCDEFabcdef中 ')
        assert.equal(keyText(char.codePointAt(0)), char);
    assert.equal(keyText('1'), keyText(49));
    assert.equal(keyText('0'), keyText(48));
});

test('keypad digits and decimal work even when the map has no Unicode value', () => {
    for (const unicode of ['', '\0', 0]) {
        for (let digit = 0; digit <= 9; digit++)
            assert.equal(keyText(unicode, 0xffb0 + digit), String(digit));
        assert.equal(keyText(unicode, 0xffae), '.');
    }
});

test('non-text keys, control characters and malformed values do not become input', () => {
    for (const unicode of ['', '\0', '\r', '\n', '\t', '\x7f', '\x85', '\ud800', '12',
        0, -1, NaN, Infinity, 0x110000, 0xd800, null, undefined])
        assert.equal(keyText(unicode), '');
    assert.equal(keyText(0, 0xff1b), '');
    assert.equal(keyText(0, 0xff51), '');
});

test('single Unicode characters remain available to the process-name editor', () => {
    assert.equal(keyText('中'), '中');
    assert.equal(keyText(' '), ' ');
    assert.equal(keyText('😀'), '😀');
    assert.equal(keyText(0x1f600), '😀');
});
