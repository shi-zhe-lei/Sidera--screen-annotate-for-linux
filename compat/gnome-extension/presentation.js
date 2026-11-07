// Keep this classifier aligned with the X11 backend. An integrated WPS
// window can switch between Writer and Presentation without changing WM_CLASS.
export function isWpsPresentation(instance, windowClass, title) {
    const classes = [instance, windowClass].map(value => String(value ?? '').toLowerCase());
    if (classes.some(value => ['wpp', 'wps-office-wpp', 'com.kingsoft.wps.wpp'].includes(value)))
        return true;
    if (!classes.some(value => ['wpsoffice', 'wps-office', 'com.kingsoft.wpsoffice', 'wps'].includes(value)))
        return false;
    const document = String(title ?? '').trim().toLowerCase()
        .replace(/ - (wps office|wps presentation|wps 演示|wps演示)$/, '')
        .replace(/[ *]+$/, '');
    return /\.(dps|dpt|ppt|pptx|pptm|pps|ppsx|ppsm|pot|potx|potm)$/.test(document) ||
        /^(演示文稿|presentation)\s*\d+$/.test(document);
}
