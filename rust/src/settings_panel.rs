//! Compact settings forms; every visible control shares geometry with hit testing.
use crate::{app::{App, now_ms}, settings_state::{self as state, DialogHit, PickerHit, Tab},
    text::Fonts, ui::{self, Ctx, R, SetHit, SliderDef, SliderId},
    ui_theme::{self as theme, c, alpha, Accent, Preference}, ui_icons::{self as icons, Glyph},
    value_input::{self, Target}};
use tiny_skia::{Pixmap, PixmapPaint, Transform};

fn inset(r:R,n:i32)->R {R {x:r.x+n,y:r.y+n,w:(r.w-n*2).max(1),h:(r.h-n*2).max(1)}}
pub fn available_size(app: &App) -> (i32, i32) {
    app.settings_ui.window_size().unwrap_or((app.screen_w, app.screen_h))
}

pub fn rect(app:&App)->R {
    if let Some((w, h)) = app.settings_ui.window_size() {
        return R { x: 0, y: 0, w, h };
    }
    let w=860.min((app.screen_w-24).max(1));let h=620.min((app.screen_h-24).max(1));
    R {x:(app.screen_w-w)/2,y:(app.screen_h-h)/2,w,h}
}
fn has_sidebar(app:&App)->bool {
    let p=rect(app);p.w>=680&&p.h>=400
}
pub fn content_rect(app:&App)->R {
    let p=rect(app);let wide=has_sidebar(app);
    let left=if wide {192}else{16};let top=if wide {94}else{90};
    R {x:p.x+left,y:p.y+top,w:p.w-left-20,h:(p.h-top-64).max(40)}
}
pub fn tab_rect(app:&App,index:usize)->R {
    let p=rect(app);
    if has_sidebar(app) {R {x:p.x+16,y:p.y+94+index as i32*46,w:136,h:38}}
    else {let w=(p.w-32)/4;R {x:p.x+16+index as i32*w,y:p.y+48,w:w-2,h:38}}
}
fn quit_rect(app:&App)->R {
    let p=rect(app);
    if has_sidebar(app) {R {x:p.x+16,y:p.y+p.h-96,w:136,h:44}}
    else {R {x:p.x+p.w-136,y:p.y+8,w:120,h:36}}
}
fn count(tab:Tab)->usize {match tab {Tab::Appearance=>5,Tab::General=>7,Tab::Addins=>3,Tab::About=>5}}
fn addins_notice_rect(app:&App)->Option<R> {
    if app.settings_ui.tab!=Tab::Addins {return None;}
    let body=content_rect(app);
    let h:i32=if body.w<460 {96}else{76};
    let h=h.min(body.h-56);
    (h>=36).then_some(R {h,..body})
}
fn rows_rect(app:&App)->R {
    let body=content_rect(app);
    if let Some(notice)=addins_notice_rect(app) {
        R {y:body.y+notice.h+12,h:body.h-notice.h-12,..body}
    } else {body}
}
pub fn layout(app:&App)->ui::SettingsLayout {
    let body=rows_rect(app);let stride=if body.w<460 {
        if app.settings_ui.tab==Tab::About {78}else{64}
    }else{54};
    let rows=(body.h/stride).max(1) as usize;let pages=count(app.settings_ui.tab).div_ceil(rows);
    ui::SettingsLayout {rows,pages,page:app.settings_page.min(pages-1),stride:stride.min(body.h)}
}
fn row(app:&App,index:usize)->Option<R> {
    let body=rows_rect(app);let l=layout(app);
    if !(l.page*l.rows..(l.page+1)*l.rows).contains(&index){return None;}
    Some(R {x:body.x,y:body.y+(index-l.page*l.rows) as i32*l.stride,w:body.w,h:l.stride})
}
pub fn change_page(app:&mut App,delta:i32)->bool {
    let l=layout(app);let next=(l.page as i64+delta as i64).clamp(0,l.pages as i64-1) as usize;
    app.settings_page=next;if next==l.page{return false;}
    app.settings_ui.nav_from=app.settings_ui.nav_position();app.settings_ui.tab_at=now_ms();app.press=None;true
}
pub fn controls(app:&App)->Vec<(SetHit,R)> {
    let p=rect(app);let mut out=Vec::new();
    for tab in Tab::ALL {out.push((SetHit::Tab(tab),tab_rect(app,tab.index())));}
    match app.settings_ui.tab {
        Tab::Appearance=>{
            if let Some(r)=row(app,0) {
                let width=if r.w>=460 {294}else{r.w-16};let x=r.x+r.w-width-8;
                for (i,preference) in [Preference::Light,Preference::Dark,Preference::System].into_iter().enumerate() {
                    out.push((SetHit::Theme(preference),R {x:x+i as i32*(width/3),y:r.y+if r.w>=460 {(r.h-40)/2}else{r.h-40},w:width/3-2,h:40}));
                }
            }
            if let Some(r)=row(app,1){out.push((SetHit::ThemeAccent,r));}
        }
        Tab::General=>for (hit,i) in [(SetHit::Autostart,0),(SetHit::RightClick,3),(SetHit::ProcessBlock,6)] {if let Some(r)=row(app,i){out.push((hit,r));}},
        Tab::Addins=>for (hit,i) in [(SetHit::WpsRegister,1),(SetHit::WpsRemove,2)] {if let Some(r)=row(app,i){out.push((hit,r));}},
        Tab::About=>{
            for (hit,i) in [(SetHit::License,2),(SetHit::Repository,3)] {if let Some(r)=row(app,i){out.push((hit,r));}}
            if let Some(r)=row(app,4) {
                out.push((SetHit::Logs,R {w:r.w/2,..r}));
                out.push((SetHit::SysInfo,R {x:r.x+r.w/2,w:r.w-r.w/2,..r}));
            }
        }
    }
    let footer=p.y+p.h-52;
    out.push((SetHit::Quit,quit_rect(app)));
    out.push((SetHit::Done,R {x:p.x+p.w-96,y:footer,w:76,h:40}));
    if layout(app).pages>1 {
        for (delta,dx) in [(-1,0),(1,42)] {out.push((SetHit::SettingsPage(delta),R {x:content_rect(app).x+dx,y:footer,w:38,h:40}));}
    }
    for s in sliders(app){out.push((SetHit::EditSlider(s.id),s.badge));out.push((SetHit::ResetSlider(s.id),s.reset));}
    out
}
pub fn sliders(app:&App)->Vec<SliderDef> {
    let values=match app.settings_ui.tab {
        Tab::Appearance=>vec![(SliderId::Alpha,2,30.0,255.0,app.sidebar_alpha as f32,1.0,"工具栏不透明度",false),
            (SliderId::Scale,3,0.6,1.4,app.sb_scale as f32,0.01,"工具栏大小",true),
            (SliderId::Position,4,0.0,100.0,value_input::position(app),1.0,"工具栏垂直位置",false)],
        Tab::General=>vec![(SliderId::StartupDelay,1,5.0,20.0,app.startup_delay as f32,1.0,"自动启动延迟",false),
            (SliderId::AutoExpand,2,1.0,20.0,app.sidebar_expand_minutes as f32,1.0,"自动展开时间",false),
            (SliderId::Threshold,4,20.0,260.0,app.large_touch_threshold as f32,10.0,"大触点阈值",false),
            (SliderId::EraseScale,5,0.8,2.5,app.large_erase_scale10 as f32/10.0,0.1,"大触点橡皮倍率",true)],
        _=>Vec::new(),
    };
    values.into_iter().filter_map(|(id,index,min,max,value,step,label,signed)| {
        let r=row(app,index)?;let compact=r.w<460;let y=r.y+if compact {(r.h-44).max(0)}else{(r.h-44)/2};
        let reset=R {x:r.x+r.w-40,y,w:32,h:44};
        let badge=R {x:reset.x-74,y,w:66,h:44};
        let start=if compact {r.x+14}else{r.x+r.w-282};
        let track=R {x:start,y:y+21,w:(badge.x-start-16).max(1),h:3};
        Some(SliderDef {id,track,hit:R {x:track.x-6,y,w:track.w+12,h:44},badge,reset,min,max,value,step,label,signed})
    }).collect()
}
pub fn fitted(fonts:&Fonts,text:&str,px:f32,width:i32)->String {
    if fonts.text_width(text,px)<=width.max(1) as f32{return text.into();}
    let mut s=text.to_string();while !s.is_empty()&&fonts.text_width(&format!("{s}…"),px)>width.max(1) as f32{s.pop();}format!("{s}…")
}
fn text(ctx:&mut Ctx,fonts:&Fonts,value:&str,r:R,px:f32,rgb:u32) {
    ctx.text(fonts,&fitted(fonts,value,px,r.w),r.x as f32,r.y as f32+(r.h as f32+px*0.72)/2.0,px,c(rgb));
}
pub fn wrapped(fonts:&Fonts,value:&str,px:f32,width:i32)->Vec<String> {
    let mut out=Vec::new();for paragraph in value.lines(){let mut line=String::new();for ch in paragraph.chars(){let mut next=line.clone();next.push(ch);if !line.is_empty()&&fonts.text_width(&next,px)>width.max(1) as f32 {out.push(line);line=String::new();}line.push(ch);}out.push(line);}out
}
pub fn button(ctx:&mut Ctx,fonts:&Fonts,r:R,label:&str,primary:bool) {
    let p=ctx.palette;let visual=R {y:r.y+(r.h-r.h.min(32))/2,h:r.h.min(32),..r};
    ui::surface(ctx,visual,6.0,c(if primary {p.accent}else{p.paper}),c(if primary {p.accent}else{p.border}));
    ctx.text_center(fonts,visual,&fitted(fonts,label,14.0,r.w-10),14.0,c(if primary {p.on_accent}else{p.text}));
}
pub fn toggle(ctx:&mut Ctx,app:&App,r:R,control:ui::Control,on:bool) {
    let p=ctx.palette;let t=app.settings_ui.toggle_value(control,on);
    let sw=R {x:r.x+r.w-50,y:r.y+(r.h-20)/2,w:34,h:20};
    ctx.fill_round(sw,10.0,c(theme::mix(p.border,p.accent,t)));
    ctx.fill_circle((sw.x+10) as f32+14.0*t,(sw.y+10) as f32,7.0,c(if t>0.5 {p.on_accent}else{p.paper}));
}
fn card(ctx:&mut Ctx,fonts:&Fonts,r:R,title:&str,detail:&str,trailing:i32) {
    let p=ctx.palette;let x=r.x+14;let w=r.w-28-trailing;
    if detail.is_empty() || r.h<46 {text(ctx,fonts,title,R {x,w,..r},15.0,p.text);}
    else {text(ctx,fonts,title,R {x,y:r.y+(r.h-42)/2,w,h:23},15.0,p.text);text(ctx,fonts,detail,R {x,y:r.y+(r.h-42)/2+23,w,h:19},12.0,p.muted);}
}
fn wps_exit_notice(ctx:&mut Ctx,fonts:&Fonts,r:R) {
    let p=ctx.palette;
    ui::surface(ctx,r,8.0,c(p.warning_light),alpha(p.warning,65));
    let compact=r.h<64;
    let top=if compact {(r.h-22)/2}else{10};
    icons::draw(ctx,Glyph::Exit,(r.x+14) as f32,(r.y+top+2) as f32,18.0,c(p.warning),c(p.warning));
    let title=R {x:r.x+44,y:r.y+top,w:r.w-58,h:22};
    text(ctx,fonts,"操作前请退出 WPS",title,15.0,p.text);
    if !compact {
        let lines=wrapped(fonts,"请先保存课件并完全退出 WPS；修改完成后重新打开。",12.0,title.w);
        for (i,line) in lines.iter().take(((r.h-40)/16).max(0) as usize).enumerate() {
            text(ctx,fonts,line,R {y:r.y+34+i as i32*16,h:16,..title},12.0,p.text);
        }
    }
}
fn contributors(ctx:&mut Ctx,fonts:&Fonts,r:R) {
    let palette=ctx.palette;
    let people=[("Carl_Jin","项目作者"),("Gary_6Stones","功能与兼容性开发"),("ofkuyu","教程与体验顾问")];
    let inner=inset(r,8);
    for (index,(name,role)) in people.into_iter().enumerate() {
        let left=inner.x+inner.w*index as i32/3;
        let right=inner.x+inner.w*(index as i32+1)/3;
        let width=(right-left-12).max(1);
        let lines=wrapped(fonts,role,12.0,width);
        let top=r.y+(r.h-22-16*lines.len().min(2) as i32)/2;
        if index>0 {
            ctx.stroke_line((left as f32,(r.y+12) as f32),(left as f32,(r.y+r.h-12) as f32),alpha(palette.border,105),1.0);
        }
        ctx.text_center(fonts,R {x:left+6,y:top,w:width,h:22},&fitted(fonts,name,15.0,width),15.0,c(palette.text));
        for (line,label) in lines.iter().take(2).enumerate() {
            ctx.text_center(fonts,R {x:left+6,y:top+22+line as i32*16,w:width,h:16},label,12.0,c(palette.muted));
        }
    }
}
fn opacity(app:&App)->f32 {state::ease(app.settings_ui.opened_at,150)*if app.settings_ui.closing_at==0 {1.0}else{1.0-state::ease(app.settings_ui.closing_at,120)}}
pub fn layer(ctx:&mut Ctx,opacity:f32,draw:impl FnOnce(&mut Ctx)) {
    if opacity>=0.999 {draw(ctx);return;}if opacity<=0.001{return;}
    if let Some(mut tile)=Pixmap::new(ctx.pm.width(),ctx.pm.height()) {
        draw(&mut Ctx {pm:&mut tile,palette:ctx.palette,tr:ctx.tr});
        ctx.pm.draw_pixmap(0,0,tile.as_ref(),&PixmapPaint {opacity,..Default::default()},Transform::identity(),None);
    }
}
fn draw_icon(ctx:&mut Ctx,icon:Option<&Pixmap>,x:i32,y:i32,size:f32) {
    if let Some(icon)=icon {ctx.pm.draw_pixmap(0,0,icon.as_ref(),&PixmapPaint::default(),ctx.tr.pre_translate(x as f32,y as f32).pre_scale(size/icon.width() as f32,size/icon.height() as f32),None);}
}
pub fn draw(app:&App,fonts:&Fonts,ctx:&mut Ctx,icon:Option<&Pixmap>) {layer(ctx,opacity(app),|ctx|draw_content(app,fonts,ctx,icon));}
fn draw_content(app:&App,fonts:&Fonts,ctx:&mut Ctx,icon:Option<&Pixmap>) {
    let p=rect(app);let palette=ctx.palette;let wide=has_sidebar(app);let body=content_rect(app);let l=layout(app);
    ui::panel(ctx,p,14.0,1.0);
    if wide {
        ctx.fill_round(R {x:p.x+1,y:p.y+1,w:166,h:p.h-2},13.0,c(theme::mix(palette.paper,palette.surface,0.65)));
        ctx.stroke_line(((p.x+168) as f32,(p.y+18) as f32),((p.x+168) as f32,(p.y+p.h-18) as f32),alpha(palette.border,150),1.0);
        draw_icon(ctx,icon,p.x+20,p.y+24,28.0);
        text(ctx,fonts,"Sidera",R {x:p.x+58,y:p.y+21,w:94,h:24},17.0,palette.text);
        text(ctx,fonts,"偏好设置",R {x:p.x+58,y:p.y+45,w:94,h:18},11.0,palette.muted);
        text(ctx,fonts,crate::app::VERSION,R {x:p.x+20,y:p.y+p.h-40,w:132,h:20},10.0,palette.muted);
        text(ctx,fonts,app.settings_ui.tab.label(),R {x:body.x,y:p.y+25,w:body.w,h:30},23.0,palette.text);
        text(ctx,fonts,app.settings_ui.tab.subtitle(),R {x:body.x,y:p.y+58,w:body.w,h:20},12.0,palette.muted);
    } else {text(ctx,fonts,"设置",R {x:p.x+18,y:p.y+10,w:p.w-174,h:28},20.0,palette.text);}
    let a=tab_rect(app,0);let b=tab_rect(app,1);let pos=app.settings_ui.nav_position();
    let active=R {x:a.x+((b.x-a.x) as f32*pos).round() as i32,y:a.y+((b.y-a.y) as f32*pos).round() as i32,..a};
    ctx.fill_round(active,6.0,c(theme::mix(palette.paper,palette.accent_light,0.62)));
    for tab in Tab::ALL {
        let r=tab_rect(app,tab.index());
        if wide {
            let glyph=match tab {Tab::Appearance=>Glyph::Pen,Tab::General=>Glyph::Settings,Tab::Addins=>Glyph::Document,Tab::About=>Glyph::Info};
            icons::draw(ctx,glyph,(r.x+12) as f32,(r.y+11) as f32,16.0,c(palette.muted),c(palette.muted));
            text(ctx,fonts,tab.label(),R {x:r.x+40,w:r.w-48,..r},14.0,palette.text);
        } else {ctx.text_center(fonts,r,tab.label(),13.0,c(palette.text));}
    }
    let visible=(count(app.settings_ui.tab)-l.page*l.rows).min(l.rows);
    let group=R {h:visible as i32*l.stride,..rows_rect(app)};
    layer(ctx,state::ease(app.settings_ui.tab_at,130),|ctx| {
        if let Some(notice)=addins_notice_rect(app) {wps_exit_notice(ctx,fonts,notice);}
        ui::surface(ctx,group,8.0,c(palette.paper),alpha(palette.border,180));
        for index in l.page*l.rows..l.page*l.rows+visible {
            let Some(r)=row(app,index) else {continue};
            if index>l.page*l.rows {ctx.stroke_line(((r.x+14) as f32,r.y as f32),((r.x+r.w-14) as f32,r.y as f32),alpha(palette.border,105),1.0);}
            match (app.settings_ui.tab,index) {
                (Tab::Appearance,0)=>{if r.w>=460 {card(ctx,fonts,r,"显示模式","浅色、深色或跟随系统",306);}else{text(ctx,fonts,"显示模式",R {x:r.x+12,y:r.y,w:r.w-24,h:23},13.0,palette.muted);}},
                (Tab::Appearance,1)=>{
                    card(ctx,fonts,r,"主题色","调色盘 · 保留你的自定义颜色",122);
                    let sw=R {x:r.x+r.w-116,y:r.y+(r.h-28)/2,w:102,h:28};
                    ui::surface(ctx,sw,5.0,c(palette.surface),c(palette.border));
                    ctx.fill_round(R {x:sw.x+7,y:sw.y+6,w:16,h:16},4.0,c(app.theme.accent.rgb()));
                    text(ctx,fonts,&format!("#{:06X}",app.theme.accent.rgb()),R {x:sw.x+29,w:sw.w-32,..sw},11.0,palette.text);
                }
                (Tab::General,0)=>{card(ctx,fonts,r,"开机自动启动","登录桌面后启动",62);toggle(ctx,app,r,ui::Control::Setting(SetHit::Autostart),app.settings_ui.autostart);},
                (Tab::General,3)=>{card(ctx,fonts,r,"右键切回光标","书写时恢复桌面操作",62);toggle(ctx,app,r,ui::Control::Setting(SetHit::RightClick),app.right_click_cursor_on);},
                (Tab::General,6)=>card(ctx,fonts,r,"进程屏蔽  ›","选择需要暂停的进程",0),
                (Tab::Addins,0)=>{card(ctx,fonts,r,"WPS 联动","始终开启，打开 WPS 后自动连接",94);text(ctx,fonts,if app.wps_connected {"● 已连接"}else{"○ 等待连接"},R {x:r.x+r.w-98,w:84,..r},12.0,palette.muted);},
                (Tab::Addins,1)=>card(ctx,fonts,r,"重新登记加载项  ›","修复当前用户的 WPS 加载项登记",0),
                (Tab::Addins,2)=>card(ctx,fonts,r,"清除加载项登记  ›","停止放映联动，可重新登记恢复",0),
                (Tab::About,0)=>{draw_icon(ctx,icon,r.x+14,r.y+(r.h-32)/2,32.0);card(ctx,fonts,R {x:r.x+44,w:r.w-44,..r},"Sidera · 星罗",crate::app::VERSION,0);},
                (Tab::About,1)=>contributors(ctx,fonts,r),
                (Tab::About,2)=>card(ctx,fonts,r,"GNU GPL v3  ↗","Copyright © 2026 Carl_Jin · 本软件不提供担保",0),
                (Tab::About,3)=>card(ctx,fonts,r,"GitHub 仓库  ↗","offduckling4825 / Sidera--screen-annotate-for-linux",0),
                _=>{},
            }
        }
        for s in sliders(app) {
            let spec=value_input::spec(app,Target::Setting(s.id));let compact=body.w<460;
            let detail=match s.id {SliderId::AutoExpand=>"收起后等待 1–20 分钟，默认 2 分钟".into(),SliderId::Position=>"从顶部到下方 · 也可拖动工具栏".into(),_=>format!("{}–{} {}",spec.min,spec.max,spec.unit)};
            let index=match s.id {SliderId::Alpha=>2,SliderId::Scale=>3,SliderId::Position=>4,SliderId::StartupDelay=>1,SliderId::AutoExpand=>2,SliderId::Threshold=>4,SliderId::EraseScale=>5};
            let r=row(app,index).unwrap();
            if compact {text(ctx,fonts,s.label,R {x:r.x+12,y:r.y,w:r.w-24,h:22},13.0,palette.text);}else{card(ctx,fonts,r,s.label,&detail,286);}
            ctx.fill_round(s.track,1.5,c(palette.border));let x=s.track.x+(s.track.w as f32*s.t()).round() as i32;
            ctx.fill_round(R {w:(x-s.track.x).max(1),..s.track},1.5,c(palette.accent));
            let cy=(s.track.y+1) as f32;
            if app.press.is_some_and(|p|p.active&&p.control==ui::Control::Slider(s.id)){ctx.fill_circle(x as f32,cy,12.0,alpha(palette.accent,24));}
            ctx.fill_circle(x as f32,cy,6.0,c(palette.paper));ctx.stroke_round(R {x:x-6,y:cy as i32-6,w:12,h:12},6.0,c(palette.accent),1.5);
            value_input::draw_value(app,fonts,ctx,Target::Setting(s.id),s.badge,13.0,"",spec.unit);
        }
        for (hit,r) in controls(app) {match hit {
            SetHit::Theme(preference)=>{
                let selected=preference==app.theme.preference;let r=R {y:r.y+5,h:r.h-10,..r};
                ui::surface(ctx,r,5.0,c(if selected {palette.accent_light}else{palette.surface}),c(if selected {palette.accent_line}else{palette.border}));
                ctx.text_center(fonts,r,preference.label(),13.0,c(palette.text));
            }
            SetHit::ResetSlider(_)=>{icons::draw(ctx,Glyph::Reset,(r.x+(r.w-16)/2) as f32,(r.y+(r.h-16)/2) as f32,16.0,c(palette.muted),c(palette.muted));}
            SetHit::Logs=>card(ctx,fonts,r,"运行日志  ›","查看与清理记录",0),
            SetHit::SysInfo=>{ctx.stroke_line((r.x as f32,(r.y+12) as f32),(r.x as f32,(r.y+r.h-12) as f32),alpha(palette.border,130),1.0);card(ctx,fonts,r,"系统诊断  ›","环境与联动状态",0);}
            _=>{},
        }}
    });
    ctx.stroke_line((body.x as f32,(p.y+p.h-62) as f32),((p.x+p.w-20) as f32,(p.y+p.h-62) as f32),alpha(palette.border,110),1.0);
    for (hit,r) in controls(app) {match hit {
        SetHit::Quit=>{
            if wide {ctx.stroke_line(((r.x+4) as f32,(r.y-12) as f32),((r.x+r.w-4) as f32,(r.y-12) as f32),alpha(palette.border,150),1.0);}
            let visual=R {y:r.y+(r.h-32)/2,h:32,..r};
            ui::surface(ctx,visual,6.0,c(palette.danger_light),alpha(palette.danger,90));
            icons::draw(ctx,Glyph::Exit,(r.x+10) as f32,(r.y+(r.h-18)/2) as f32,18.0,c(palette.danger),c(palette.danger));
            text(ctx,fonts,"退出 Sidera",R {x:r.x+36,w:r.w-42,..r},14.0,palette.danger);
        }
        SetHit::Done=>button(ctx,fonts,r,"完成",true),
        SetHit::SettingsPage(delta)=>{ctx.text_center(fonts,r,if delta<0 {"‹"}else{"›"},20.0,c(if ui::control_enabled(app,ui::Control::Setting(hit)){palette.text}else{palette.disabled_ink}));}
        _=>{},
    }}
    let editing=app.settings_ui.editor.as_ref().filter(|e|matches!(e.target,Target::Setting(_)));
    let caption=if let Some(editor)=editing {value_input::message(app,editor.target).unwrap_or_default()}
        else if app.settings_ui.toast_until>now_ms(){app.settings_ui.toast.clone()}
        else if l.pages>1{format!("{} / {}",l.page+1,l.pages)}
        else if app.settings_ui.tab==Tab::Addins {"登记变更后需重新打开 WPS".into()}
        else{"更改即时生效 · 点击数值可输入".into()};
    let dx=if l.pages>1{90}else{0};
    text(ctx,fonts,&caption,R {x:body.x+dx,y:p.y+p.h-50,w:(body.w-96-dx).max(1),h:36},11.0,
        if editing.is_some_and(|e|!e.error.is_empty()){palette.danger}else{palette.muted});
}

pub fn dialog_rect(app: &App) -> R {
    let (width, height) = available_size(app);
    let wps = app.settings_ui.dialog.is_some_and(state::Dialog::requires_wps_exit);
    let preferred_w:i32 = if wps {480}else{460};
    let w = preferred_w.min((width - 24).max(1));
    let desired_h:i32 = if wps {if w<400 {332}else{280}}else{260};
    let h = desired_h.min((height - 24).max(1));
    R { x: (width - w) / 2, y: (height - h) / 2, w, h }
}
pub fn dialog_controls(app: &App) -> Vec<(DialogHit, R)> {
    let p = dialog_rect(app); let w = (p.w - 52) / 2;
    vec![(DialogHit::Cancel, R { x: p.x + 20, y: p.y + p.h - 64, w, h: 44 }),
        (DialogHit::Continue, R { x: p.x + 32 + w, y: p.y + p.h - 64, w, h: 44 })]
}
pub fn picker_rect(app: &App) -> R {
    let (width, height) = available_size(app);
    let w = 440.min((width - 24).max(1)); let h = 448.min((height - 24).max(1));
    R { x: (width - w) / 2, y: (height - h) / 2, w, h }
}
pub const PRESETS: [Accent; 8] = [Accent::Teal, Accent::Blue, Accent::Green, Accent::Amber, Accent::Violet, Accent::Rose, Accent::Custom(0x40444F), Accent::Custom(0xEE7942)];
pub fn picker_controls(app:&App)->Vec<(PickerHit,R)> {
    let p=picker_rect(app);let compact=p.h<360;
    let field=R {x:p.x+20,y:p.y+if compact {50}else{54},w:p.w-40,h:(p.h-if compact {210}else{276}).max(24)};
    let hue=R {x:field.x,y:field.y+field.h+12,w:field.w,h:20};
    let mut out=vec![(PickerHit::Field,field),(PickerHit::Hue,hue)];
    let hex_width=(field.w*34/100).max(84);let small=(field.w-hex_width-18)/3;
    out.push((PickerHit::Edit(Target::Hex),R {x:field.x,y:hue.y+30,w:hex_width,h:36}));
    for (i,target) in [Target::Hue,Target::Saturation,Target::Brightness].into_iter().enumerate(){
        out.push((PickerHit::Edit(target),R {x:field.x+hex_width+6+i as i32*(small+6),y:hue.y+30,w:small,h:36}));
    }
    if !compact {
        let unit=field.w/8;
        for i in 0..8 {out.push((PickerHit::Preset(i),R {x:field.x+i as i32*unit,y:hue.y+72,w:unit-2,h:36}));}
        if p.h>=430 {for i in 0..app.settings_ui.recent.len().min(8){out.push((PickerHit::Recent(i),R {x:field.x+i as i32*unit,y:hue.y+112,w:unit-2,h:36}));}}
    }
    let w=(p.w-52)/2;
    out.push((PickerHit::Cancel,R {x:p.x+20,y:p.y+p.h-54,w,h:40}));
    out.push((PickerHit::Apply,R {x:p.x+32+w,y:p.y+p.h-54,w,h:40}));out
}
pub fn picker_drag(app: &mut App, hit: PickerHit, x: i32, y: i32) {
    let Some((_, r)) = picker_controls(app).into_iter().find(|(k, _)| *k == hit) else { return };
    let Some(draft) = app.settings_ui.picker.as_mut() else { return };
    match hit {
        PickerHit::Field => { draft.saturation = ((x-r.x) as f32 / r.w.max(1) as f32).clamp(0.0,1.0); draft.value = 1.0 - ((y-r.y) as f32 / r.h.max(1) as f32).clamp(0.0,1.0); }
        PickerHit::Hue => draft.hue = ((x-r.x) as f32 / r.w.max(1) as f32).clamp(0.0,0.9999),
        _ => {}
    }
}
pub fn draw_overlay(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    if app.settings_ui.dialog.is_none() && app.settings_ui.picker.is_none() { return; }
    let opacity = state::ease(app.settings_ui.overlay_at, 160) * if app.settings_ui.overlay_closing_at == 0 { 1.0 } else { 1.0 - state::ease(app.settings_ui.overlay_closing_at, 120) };
    layer(ctx, opacity, |ctx| {
        let palette = ctx.palette;
        let (width, height) = available_size(app);
        ctx.fill_round(R { x: 0, y: 0, w: width, h: height }, 0.0, alpha(0, if app.theme.dark() { 140 } else { 80 }));
        if let Some(dialog) = app.settings_ui.dialog {
            let p = dialog_rect(app); ui::panel(ctx, p, 12.0, 1.0);
            let body_bottom = if dialog.requires_wps_exit() {
                let ink=if dialog.destructive() {palette.danger}else{palette.warning};
                let fill=if dialog.destructive() {palette.danger_light}else{palette.warning_light};
                ctx.fill_round(R {x:p.x+24,y:p.y+20,w:32,h:32},8.0,c(fill));
                icons::draw(ctx,Glyph::Warning,(p.x+30) as f32,(p.y+26) as f32,20.0,c(ink),c(ink));
                let px=if p.w<400 {16.0}else{18.0};
                let title=wrapped(fonts,dialog.title(),px,p.w-108);
                let top=p.y+20+(32-title.len().min(2) as i32*20)/2;
                for (i,line) in title.iter().take(2).enumerate() {
                    text(ctx,fonts,line,R {x:p.x+68,y:top+i as i32*20,w:p.w-92,h:20},px,palette.text);
                }
                let h=if p.h<260 {36}else if p.w<400 {96}else{72};
                let notice=R {x:p.x+24,y:p.y+p.h-76-h,w:p.w-48,h};
                wps_exit_notice(ctx,fonts,notice);
                notice.y-8
            } else {
                text(ctx,fonts,dialog.title(),R {x:p.x+24,y:p.y+14,w:p.w-48,h:36},18.0,palette.text);
                p.y+p.h-76
            };
            let lines=wrapped(fonts,dialog.body(),14.0,p.w-48);
            let body_top=p.y+62;
            for (i,line) in lines.iter().take(((body_bottom-body_top)/20).max(0) as usize).enumerate() {
                text(ctx,fonts,line,R {x:p.x+24,y:body_top+i as i32*20,w:p.w-48,h:20},14.0,palette.text);
            }
            for (hit, r) in dialog_controls(app) {
                let emphasis=if dialog.destructive() {Some((palette.danger,palette.danger_light))}
                    else if dialog==state::Dialog::Addins {Some((palette.warning,palette.warning_light))}else{None};
                if let Some((ink,fill))=emphasis.filter(|_|hit==DialogHit::Continue) {
                    let r=R {y:r.y+6,h:r.h-12,..r};
                    ui::surface(ctx,r,6.0,c(fill),alpha(ink,140));
                    ctx.text_center(fonts,r,&fitted(fonts,dialog.action(),14.0,r.w-12),14.0,c(palette.text));
                } else { button(ctx, fonts, r, if hit == DialogHit::Cancel { "取消" } else { dialog.action() }, hit == DialogHit::Continue); }
            }
        } else if let Some(draft) = &app.settings_ui.picker {
            let p = picker_rect(app); ui::panel(ctx, p, 12.0, 1.0);
            if let Some(editor)=app.settings_ui.editor.as_ref() {
                text(ctx,fonts,&value_input::message(app,editor.target).unwrap_or_default(),
                    R {x:p.x+22,y:p.y+8,w:p.w-44,h:40},12.0,if editor.error.is_empty(){palette.muted}else{palette.danger});
            } else {
                text(ctx, fonts, "主题色", R { x: p.x + 22, y: p.y + 8, w: p.w - 160, h: 40 }, 18.0, palette.text);
                text(ctx, fonts, &format!("#{:06X}", draft.rgb()), R { x: p.x + p.w - 130, y: p.y + 8, w: 112, h: 40 }, 13.0, palette.text);
            }
            for (hit, r) in picker_controls(app) {
                match hit {
                    PickerHit::Edit(target) => {
                        let label=match target {Target::Hue=>"H ",Target::Saturation=>"S ",Target::Brightness=>"V ",_=>""};
                        value_input::draw_value(app,fonts,ctx,target,r,12.0,label,"");
                    }
                    PickerHit::Field => {
                        let cols = 32; let rows = 24;
                        for y in 0..rows { for x in 0..cols {
                            let tile = R { x: r.x + r.w*x/cols, y: r.y+r.h*y/rows, w: r.w*(x+1)/cols-r.w*x/cols+1, h: r.h*(y+1)/rows-r.h*y/rows+1 };
                            ctx.fill_round(tile, 0.0, c(state::hsv(draft.hue, x as f32/(cols-1) as f32, 1.0-y as f32/(rows-1) as f32)));
                        } }
                        let x = r.x + (draft.saturation * (r.w-1) as f32) as i32;
                        let y = r.y + ((1.0-draft.value) * (r.h-1) as f32) as i32;
                        ctx.stroke_round(R { x: x-7, y: y-7, w: 14, h: 14 }, 7.0, c(0xFFFFFF), 2.0);
                        ctx.stroke_round(R { x: x-9, y: y-9, w: 18, h: 18 }, 9.0, c(0x25262B), 1.0);
                    }
                    PickerHit::Hue => {
                        for i in 0..72 { ctx.fill_round(R { x: r.x+r.w*i/72, w: r.w*(i+1)/72-r.w*i/72+1, ..r }, 0.0, c(state::hsv(i as f32/72.0,1.0,1.0))); }
                        ctx.stroke_round(R { x: r.x+(r.w as f32*draft.hue) as i32-5, y:r.y-3, w:10, h:r.h+6 }, 4.0, c(palette.text), 2.0);
                    }
                    PickerHit::Preset(i) | PickerHit::Recent(i) => {
                        let rgb = if matches!(hit, PickerHit::Preset(_)) { PRESETS[i].rgb() } else { app.settings_ui.recent[i] };
                        ctx.fill_circle((r.x+r.w/2) as f32,(r.y+r.h/2) as f32,11.0,c(rgb));
                        if rgb == draft.rgb() { icons::draw(ctx,Glyph::Check,(r.x+r.w/2-10) as f32,(r.y+r.h/2-10) as f32,20.0,theme::on_color(c(rgb)),c(rgb)); }
                    }
                    PickerHit::Cancel => button(ctx,fonts,r,"取消",false),
                    PickerHit::Apply => {
                        let r=R {y:r.y+4,h:r.h-8,..r};
                        ui::surface(ctx,r,6.0,c(draft.rgb()),c(palette.border));
                        ctx.text_center(fonts,r,"应用颜色",14.0,theme::on_color(c(draft.rgb())));
                    }
                }
            }
        }
    });
}

fn inspector_pager_rect(app:&App)->R {
    let p=ui::diag_rect(app);
    let stacked=p.w<320;
    let available=if stacked {p.w-36}else{p.w-148};
    let w=if available>=280 {280}else{172.min(available.max(1))};
    R {x:p.x+18,y:p.y+p.h-if stacked {106}else{62},w,h:44}
}
fn inspector_page_button(app:&App,delta:i32)->R {
    let r=inspector_pager_rect(app);
    let w=if r.w>=280 {88}else{44};
    R {x:if delta<0 {r.x}else{r.x+r.w-w},w,..r}
}
pub fn inspector_body_rect(app:&App)->R {
    let p=ui::diag_rect(app);let footer=inspector_pager_rect(app);
    R {x:p.x+22,y:p.y+74,w:p.w-44,h:(footer.y-p.y-86).max(20)}
}
pub fn inspector_controls(app: &App) -> Vec<(SetHit, R)> {
    let p = ui::diag_rect(app); let mut out = Vec::new();
    for delta in [-1,1] {out.push((SetHit::InspectorPage(delta),inspector_page_button(app,delta)));}
    if app.settings_ui.inspector == state::Inspector::Logs {
        out.push((SetHit::ClearLog,R { x:p.x+p.w-102,y:p.y+12,w:84,h:44 }));
    }
    out.push((SetHit::InspectorClose,R { x:p.x+p.w-110,y:p.y+p.h-62,w:92,h:44 }));
    out
}
pub fn inspector_lines(app: &App, fonts: &Fonts) -> Vec<String> { wrapped(fonts,&app.diag_text,13.0,inspector_body_rect(app).w) }
pub fn inspector_page_size(app: &App) -> usize { (inspector_body_rect(app).h/20).max(1) as usize }
pub fn inspector_can_page(app:&App,fonts:&Fonts,delta:i32)->bool {
    let pages=inspector_lines(app,fonts).len().div_ceil(inspector_page_size(app)).max(1);
    let page=app.settings_ui.inspector_page.min(pages-1);
    (delta<0&&page>0)||(delta>0&&page+1<pages)
}
fn draw_inspector_pager(app:&App,fonts:&Fonts,ctx:&mut Ctx,page:usize,pages:usize) {
    let p=ctx.palette;let r=inspector_pager_rect(app);
    let visual=R {y:r.y+6,h:32,..r};
    ui::surface(ctx,visual,6.0,c(p.surface),c(p.border));
    let previous=inspector_page_button(app,-1);let next=inspector_page_button(app,1);
    for delta in [-1,1] {
        let hit=SetHit::InspectorPage(delta);let b=inspector_page_button(app,delta);
        let enabled=if delta<0 {page>0}else{page+1<pages};
        let pressed=enabled&&app.press.is_some_and(|press|press.active&&!press.expired()&&press.control==ui::Control::Setting(hit));
        if pressed {ctx.fill_round(R {x:b.x+2,y:visual.y+2,w:b.w-4,h:visual.h-4},4.0,c(p.accent_light));}
        let ink=if enabled {p.text}else{p.disabled_ink};
        let labeled=b.w>=88;
        let start=b.x+(b.w-68)/2;
        let x=if !labeled {b.x+(b.w-18)/2}else if delta<0 {start}else{start+50};
        icons::draw(ctx,if delta<0 {Glyph::Left}else{Glyph::Right},x as f32,(b.y+13) as f32,18.0,c(ink),c(ink));
        if labeled {
            text(ctx,fonts,if delta<0 {"上一页"}else{"下一页"},R {x:if delta<0 {start+24}else{start},w:44,..b},13.0,ink);
        }
    }
    for x in [previous.x+previous.w,next.x] {
        ctx.stroke_line((x as f32,(visual.y+8) as f32),(x as f32,(visual.y+24) as f32),alpha(p.border,150),1.0);
    }
    let counter=R {x:previous.x+previous.w+4,w:(next.x-previous.x-previous.w-8).max(1),..r};
    ctx.text_center(fonts,counter,&fitted(fonts,&format!("{} / {}",page+1,pages),13.0,counter.w),13.0,c(p.muted));
}
pub fn draw_inspector(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    let p = ui::diag_rect(app); let palette = ctx.palette;
    layer(ctx, state::ease(app.settings_ui.overlay_at,160), |ctx| {
        ui::panel(ctx,p,12.0,1.0);
        text(ctx,fonts,app.settings_ui.inspector.title(),R { x:p.x+22,y:p.y+14,w:p.w-136,h:36 },19.0,palette.text);
        let lines=inspector_lines(app,fonts); let size=inspector_page_size(app); let pages=lines.len().div_ceil(size).max(1);
        let page=app.settings_ui.inspector_page.min(pages-1);
        text(ctx,fonts,if pages>1 {"支持滚轮与上下滑动翻页"}else{"全部内容"},R { x:p.x+22,y:p.y+48,w:p.w-44,h:20 },12.0,palette.muted);
        let body=inspector_body_rect(app);
        for (i,line) in lines.iter().skip(page*size).take(size).enumerate() {
            text(ctx,fonts,line,R {y:body.y+i as i32*20,h:20,..body},13.0,
                if line.contains("[ERROR]") { palette.danger } else { palette.text });
        }
        let footer=inspector_pager_rect(app);
        ctx.stroke_line(((p.x+18) as f32,(footer.y-8) as f32),((p.x+p.w-18) as f32,(footer.y-8) as f32),alpha(palette.border,120),1.0);
        draw_inspector_pager(app,fonts,ctx,page,pages);
        for (hit,r) in inspector_controls(app) {
            let label=match hit {SetHit::InspectorPage(_)=>continue,SetHit::ClearLog=>"清空日志",_=>"关闭"};
            button(ctx,fonts,r,label,hit==SetHit::InspectorClose);
        }
    });
}

pub fn process_rect(app: &App) -> R {
    if let Some((w, h)) = app.settings_ui.window_size() {
        return R { x: 0, y: 0, w, h };
    }
    let w=660.min((app.screen_w-24).max(1)); let h=560.min((app.screen_h-24).max(1));
    R { x:(app.screen_w-w)/2,y:(app.screen_h-h)/2,w,h }
}
pub fn process_list_rect(app: &App) -> R {
    let p=process_rect(app);let top=if p.h>=440 {176}else{120};
    let footer=process_pager_rect(app);
    R {x:p.x+18,y:p.y+top,w:p.w-36,h:(footer.y-p.y-top-10).max(44)}
}
pub fn process_visible_rows(app: &App) -> usize { (process_list_rect(app).h/52).max(1) as usize }
fn process_tabs_rect(app:&App)->R {
    let p=process_rect(app);R {x:p.x+18,y:p.y+if p.h>=440 {126}else{68},w:p.w-36,h:44}
}
fn process_pager_rect(app:&App)->R {
    let p=process_rect(app);let stacked=p.w<320;
    let available=if stacked {p.w-36}else{p.w-148};
    let w=if available>=280 {280}else{172.min(available.max(1))};
    R {x:p.x+18,y:p.y+p.h-if stacked {106}else{62},w,h:44}
}
fn process_page_button(app:&App,up:bool)->R {
    let r=process_pager_rect(app);let w=if r.w>=280 {88}else{44};
    R {x:if up {r.x}else{r.x+r.w-w},w,..r}
}
pub fn process_controls(app: &App) -> Vec<(ui::ProcessHit,R)> {
    use ui::ProcessHit as H;
    let p=process_rect(app);let tabs=process_tabs_rect(app);
    let refresh=if p.w>=460 {88}else{44};let tab_width=((tabs.w-refresh-12)/2).min(132);
    let mut out=vec![(H::Enable,R {x:p.x+p.w-148,y:p.y+10,w:130,h:44}),
        (H::Done,R {x:p.x+p.w-110,y:p.y+p.h-62,w:92,h:44}),
        (H::ScrollUp,process_page_button(app,true)),(H::ScrollDown,process_page_button(app,false)),
        (H::Scan,R {x:tabs.x+tabs.w-refresh,w:refresh,..tabs})];
    for (index,view) in [state::ProcessView::Rules,state::ProcessView::Running].into_iter().enumerate() {
        out.push((H::View(view),R {x:tabs.x+index as i32*tab_width,w:tab_width,..tabs}));
    }
    if p.h>=440 {
        out.extend([(H::Input,R {x:p.x+18,y:p.y+76,w:p.w-164,h:44}),
            (H::Paste,R {x:p.x+p.w-138,y:p.y+76,w:52,h:44}),
            (H::Add,R {x:p.x+p.w-78,y:p.y+76,w:60,h:44})]);
    }
    let rows=ui::process_rows(app); let visible=process_visible_rows(app);
    let start=app.process_scroll.min(rows.len().saturating_sub(visible)); let list=process_list_rect(app);
    for (offset,(row,_,_)) in rows.iter().skip(start).take(visible).enumerate() {
        let hit=match row { ui::ProcessRow::Rule(i)=>H::Rule(*i),ui::ProcessRow::Proc(i)=>H::Proc(*i) };
        out.push((hit,R {x:list.x+1,y:list.y+offset as i32*52+1,w:list.w-8,h:list.h.min(50)}));
    }
    out
}
fn draw_process_pager(app:&App,fonts:&Fonts,ctx:&mut Ctx,start:usize,total:usize) {
    use ui::ProcessHit as H;
    let p=ctx.palette;let r=process_pager_rect(app);let visual=R {y:r.y+6,h:32,..r};
    ui::surface(ctx,visual,6.0,c(p.surface),c(p.border));
    let previous=process_page_button(app,true);let next=process_page_button(app,false);
    for (hit,up) in [(H::ScrollUp,true),(H::ScrollDown,false)] {
        let b=process_page_button(app,up);let control=ui::Control::Process(hit);
        let enabled=ui::control_enabled(app,control);
        if enabled&&app.press.is_some_and(|press|press.active&&!press.expired()&&press.control==control) {
            ctx.fill_round(R {x:b.x+2,y:visual.y+2,w:b.w-4,h:28},4.0,c(p.accent_light));
        }
        let ink=if enabled {p.text}else{p.disabled_ink};let labeled=b.w>=88;
        let x=if labeled {b.x+10}else{b.x+(b.w-18)/2};
        icons::draw(ctx,if up {Glyph::Up}else{Glyph::Down},x as f32,(b.y+13) as f32,18.0,c(ink),c(ink));
        if labeled {text(ctx,fonts,if up {"上一组"}else{"下一组"},R {x:b.x+34,w:44,..b},13.0,ink);}
    }
    for x in [previous.x+previous.w,next.x] {
        ctx.stroke_line((x as f32,(visual.y+8) as f32),(x as f32,(visual.y+24) as f32),alpha(p.border,150),1.0);
    }
    let counter=R {x:previous.x+previous.w+4,w:(next.x-previous.x-previous.w-8).max(1),..r};
    let label=if total==0 {"0 条".into()}else{format!("{}–{} / {}",start+1,(start+process_visible_rows(app)).min(total),total)};
    ctx.text_center(fonts,counter,&fitted(fonts,&label,12.0,counter.w),12.0,c(p.muted));
}
pub fn draw_process(app: &App, fonts: &Fonts, ctx: &mut Ctx) {
    use ui::ProcessHit as H;
    layer(ctx,state::ease(app.settings_ui.overlay_at,160),|ctx| {
        let p=process_rect(app); let palette=ctx.palette; ui::panel(ctx,p,12.0,1.0);
        text(ctx,fonts,"进程屏蔽",R {x:p.x+20,y:p.y+10,w:p.w-174,h:34},20.0,palette.text);
        text(ctx,fonts,"启用后暂停匹配进程，关闭屏蔽会恢复",R {x:p.x+20,y:p.y+44,w:p.w-40,h:20},12.0,palette.muted);
        let rows=ui::process_rows(app);let list=process_list_rect(app);let visible=process_visible_rows(app);
        let start=app.process_scroll.min(rows.len().saturating_sub(visible));
        let group=R {h:if rows.is_empty() {list.h}else{rows.len().min(visible) as i32*52},..list};
        ui::surface(ctx,group,8.0,c(palette.paper),alpha(palette.border,160));
        draw_process_pager(app,fonts,ctx,start,rows.len());
        for (hit,r) in process_controls(app) {
            match hit {
                H::Enable => {
                    text(ctx,fonts,if app.process_block_enabled {"已启用"}else{"未启用"},R {w:r.w-54,..r},13.0,palette.muted);
                    toggle(ctx,app,r,ui::Control::Process(H::Enable),app.process_block_enabled);
                }
                H::View(view)=>{
                    let selected=app.settings_ui.process_view==view;
                    let visual=R {x:r.x+1,y:r.y+6,w:r.w-2,h:32};
                    if selected {ctx.fill_round(visual,6.0,c(palette.accent_light));}
                    let (label,count)=if view==state::ProcessView::Rules {("已添加",app.process_rules.len())}else{("运行中",app.process_scan.len())};
                    ctx.text_center(fonts,visual,&fitted(fonts,&format!("{label}  {count}"),13.0,visual.w-8),13.0,c(if selected {palette.text}else{palette.muted}));
                }
                H::Input => {
                    let visual=R {y:r.y+4,h:36,..r};
                    ui::surface(ctx,visual,6.0,c(palette.surface),c(if app.process_input_active {palette.accent_line}else{palette.border}));
                    text(ctx,fonts,if app.process_input.is_empty() {"按进程名或路径添加"}else{&app.process_input},R {x:r.x+10,w:r.w-20,..r},13.0,if app.process_input.is_empty(){palette.muted}else{palette.text});
                }
                H::Rule(i) | H::Proc(i) => {
                    let key=if matches!(hit,H::Rule(_)) {ui::ProcessRow::Rule(i)}else{ui::ProcessRow::Proc(i)};
                    let Some((_,label,detail))=rows.iter().find(|(row,_,_)|*row==key) else {continue};
                    let selected=matches!(hit,H::Rule(_)) || app.process_scan.get(i).is_some_and(|proc|app.process_rules.iter().any(|rule|crate::process_block::matches(rule,proc)));
                    if r.y>list.y+1 {ctx.stroke_line(((r.x+12) as f32,(r.y-1) as f32),((r.x+r.w-8) as f32,(r.y-1) as f32),alpha(palette.border,90),1.0);}
                    let badge=R {x:r.x+12,y:r.y+13,w:24,h:24};
                    ctx.fill_round(badge,6.0,c(if selected {palette.accent_light}else{palette.surface}));
                    icons::draw(ctx,if selected {Glyph::Check}else{Glyph::Document},(badge.x+4) as f32,(badge.y+4) as f32,16.0,c(palette.muted),c(palette.muted));
                    text(ctx,fonts,label,R {x:r.x+46,y:r.y+4,w:r.w-116,h:23},14.0,palette.text);
                    text(ctx,fonts,detail,R {x:r.x+46,y:r.y+27,w:r.w-116,h:17},11.0,palette.muted);
                    let action=R {x:r.x+r.w-60,y:r.y+11,w:52,h:28};
                    ui::surface(ctx,action,5.0,c(palette.surface),alpha(palette.border,120));
                    ctx.text_center(fonts,action,if selected {"移除"}else{"添加"},12.0,c(if selected {palette.danger}else{palette.text}));
                }
                H::ScrollUp | H::ScrollDown=>{},
                H::Scan if r.w<60=>{
                    let visual=R {y:r.y+6,h:32,..r};
                    ui::surface(ctx,visual,6.0,c(palette.surface),c(palette.border));
                    icons::draw(ctx,Glyph::Reset,(r.x+13) as f32,(r.y+13) as f32,18.0,c(palette.text),c(palette.text));
                }
                H::Add if !ui::control_enabled(app,ui::Control::Process(hit))=>{
                    let visual=R {y:r.y+6,h:32,..r};
                    ui::surface(ctx,visual,6.0,c(palette.disabled_surface),c(palette.disabled_border));
                    ctx.text_center(fonts,visual,"添加",13.0,c(palette.disabled_ink));
                }
                _ => {
                    let label=match hit {H::Add=>"添加",H::Paste=>"粘贴",H::Scan=>"刷新列表",_=>"完成"};
                    button(ctx,fonts,r,label,matches!(hit,H::Done|H::Add));
                }
            }
        }
        if rows.is_empty() {
            let rules=app.settings_ui.process_view==state::ProcessView::Rules;
            let top=list.y+(list.h-48)/2;
            text(ctx,fonts,if rules {"还没有添加屏蔽规则"}else{"暂无运行中的进程"},R {x:list.x+18,y:top,w:list.w-36,h:24},14.0,palette.text);
            text(ctx,fonts,if rules {"从“运行中”点选添加，或输入进程名"}else{"点击刷新，重新获取当前进程"},R {x:list.x+18,y:top+26,w:list.w-36,h:20},12.0,palette.muted);
        } else if rows.len()>visible {
            let h=(group.h*visible as i32/rows.len() as i32).max(18);
            let y=group.y+(group.h-h)*start as i32/(rows.len()-visible) as i32;
            ctx.fill_round(R {x:group.x+group.w-5,y:y+2,w:2,h:(h-4).max(1)},1.0,alpha(palette.muted,120));
        }
    });
}
