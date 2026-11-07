import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';

const source = readFileSync(new URL('../presentation.js', import.meta.url), 'utf8');
const {isWpsPresentation} = await import(`data:text/javascript,${encodeURIComponent(source)}`);

test('the same integrated WPS window follows its current document type', () => {
    for (const [title, expected] of [
        ['课程.dps - WPS Office', true],
        ['笔记.pptx.docx - WPS Office', false],
        ['汇总.xlsx - WPS Office', false],
        ['演示文稿1 - WPS Office', true],
        ['WPS Office', false],
    ])
        assert.equal(isWpsPresentation('wpsoffice', 'wpsoffice', title), expected, title);
});

test('a standalone slideshow can have no title and another app is excluded', () => {
    assert.equal(isWpsPresentation('wpp', 'Wpp', ''), true);
    assert.equal(isWpsPresentation('wps', 'Wps', '文档1'), false);
    assert.equal(isWpsPresentation('firefox', 'Firefox', '课程.pptx'), false);
});
