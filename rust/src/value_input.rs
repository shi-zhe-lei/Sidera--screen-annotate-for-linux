//! Numeric/hex editing shared by settings, the colour picker and canvas controls.
use crate::{app::App, text::Fonts, ui::{self, Ctx, R, SliderId}, ui_theme::c};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target { Setting(SliderId), DotSize, Page, Hue, Saturation, Brightness, Hex }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key { Character(char), Backspace, Delete, Left, Right, Home, End, SelectAll, Paste, Enter, Escape }
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit { Field }

pub struct Spec { pub title: &'static str, pub min: f32, pub max: f32, pub digits: usize, pub unit: &'static str, pub value: f32 }
pub fn position(app: &App) -> f32 {
    let height = app.sidebar_position_height();
    let span=(app.screen_h-height).max(0);
    if span==0 {50.0} else {app.sidebar_y(height) as f32/span as f32*100.0}
}
pub fn spec(app: &App, target: Target) -> Spec {
    use Target::*;
    let (title,min,max,digits,unit,value)=match target {
        Setting(SliderId::Alpha)=>("工具栏不透明度",12.0,100.0,0,"%",app.sidebar_alpha as f32/255.0*100.0),
        Setting(SliderId::Scale)=>("工具栏大小",60.0,140.0,0,"%",app.sb_scale as f32*100.0),
        Setting(SliderId::Position)=>("工具栏垂直位置",0.0,100.0,0,"%",position(app)),
        Setting(SliderId::StartupDelay)=>("自动启动延迟",5.0,20.0,0,"秒",app.startup_delay as f32),
        Setting(SliderId::AutoExpand)=>("自动展开时间",1.0,20.0,0,"分钟",app.sidebar_expand_minutes as f32),
        Setting(SliderId::Threshold)=>("大触点阈值",20.0,260.0,0,"px",app.large_touch_threshold as f32),
        Setting(SliderId::EraseScale)=>("大触点橡皮倍率",0.8,2.5,1,"×",app.large_erase_scale10 as f32/10.0),
        DotSize=>("白板点大小",1.0,10.0,0,"",app.whiteboard_dot_size as f32),
        Page=>("跳转页码",1.0,app.page_picker.as_ref().map_or(1,|p|p.total) as f32,0,"页",app.page_picker.as_ref().map_or(1,|p|p.selected()) as f32),
        Hue=>("色相 H",0.0,360.0,1,"°",app.settings_ui.picker.as_ref().map_or(0.0,|p|p.hue*360.0)),
        Saturation=>("饱和度 S",0.0,100.0,1,"%",app.settings_ui.picker.as_ref().map_or(0.0,|p|p.saturation*100.0)),
        Brightness=>("明度 V",0.0,100.0,1,"%",app.settings_ui.picker.as_ref().map_or(0.0,|p|p.value*100.0)),
        Hex=>("主题色 HEX",0.0,0.0,0,"",0.0),
    };
    Spec {title,min,max,digits,unit,value}
}
pub fn value_text(app: &App, target: Target) -> String {
    if target==Target::Hex { return format!("#{:06X}",app.settings_ui.picker.as_ref().map_or(app.theme.accent.rgb(),|p|p.rgb())); }
    let s=spec(app,target); format!("{:.*}",s.digits,s.value)
}

pub struct Editor {
    pub target: Target, pub text: String, pub initial: String, pub cursor: usize, pub selected: bool,
    pub error: String, pub opened_at: i64,
}
impl Editor {
    pub fn new(app: &App,target: Target)->Self {
        let text=value_text(app,target); let cursor=text.len();
        Self {target,initial:text.clone(),text,cursor,selected:true,error:String::new(),opened_at:crate::app::now_ms()}
    }
    fn clear_selection(&mut self) {if self.selected {self.text.clear();self.cursor=0;self.selected=false;}}
    pub fn insert(&mut self,text:&str) {
        if !text.chars().all(|ch| if self.target==Target::Hex {ch.is_ascii_hexdigit() || ch=='#'} else {ch.is_ascii_digit() || matches!(ch,'.'|'-'|'+')}) {
            self.error="请输入数字或有效颜色代码".into();return;
        }
        let accepted:String=text.chars().take(16).collect();
        if accepted.is_empty() {return;}
        self.clear_selection();
        if self.text.len()+accepted.len()>16 {return;}
        self.text.insert_str(self.cursor,&accepted);self.cursor+=accepted.len();self.error.clear();
    }
    pub fn edit(&mut self,key:Key) {
        self.error.clear();
        match key {
            Key::Character(ch)=>self.insert(&ch.to_string()),
            Key::SelectAll=>self.selected=true,
            Key::Backspace=>{if self.selected {self.clear_selection();} else if self.cursor>0 {self.cursor-=1;self.text.remove(self.cursor);}},
            Key::Delete=>{if self.selected {self.clear_selection();} else if self.cursor<self.text.len(){self.text.remove(self.cursor);}},
            Key::Left=>{self.cursor=if self.selected {0} else {self.cursor.saturating_sub(1)};self.selected=false;},
            Key::Right=>{self.cursor=if self.selected {self.text.len()} else {(self.cursor+1).min(self.text.len())};self.selected=false;},
            Key::Home=>{self.cursor=0;self.selected=false;},
            Key::End=>{self.cursor=self.text.len();self.selected=false;},
            _=>{},
        }
    }
}
pub fn parse(app:&App)->Result<f32,String> {
    let editor=app.settings_ui.editor.as_ref().ok_or("没有正在编辑的数值")?;
    if editor.target==Target::Hex {
        let text=editor.text.trim(); let hex=text.strip_prefix('#').unwrap_or(text);
        if hex.len()!=6 {return Err("请输入 6 位颜色代码，例如 #287A70".into());}
        return u32::from_str_radix(hex,16).map(|v|v as f32).map_err(|_|"颜色代码只能包含 0–9 和 A–F".into());
    }
    let s=spec(app,editor.target);
    let value=editor.text.trim().parse::<f32>().ok().filter(|v|v.is_finite())
        .ok_or_else(||"请输入有效数字".to_string())?;
    if value<s.min || value>s.max {return Err(format!("请输入 {}–{} {}",s.min,s.max,s.unit));}
    let scale=10.0f32.powi(s.digits as i32);Ok((value*scale).round()/scale)
}
pub fn target_rect(app:&App,target:Target)->Option<R> {
    match target {
        Target::Setting(id)=>ui::settings_sliders(app).into_iter().find(|s|s.id==id).map(|s|s.badge),
        Target::DotSize=>Some(ui::whiteboard_dot_value_rect(app)),
        Target::Page=>Some(ui::page_picker_control_rect(app,ui::PagePickerHit::Input)),
        _=>crate::settings_panel::picker_controls(app).into_iter()
            .find_map(|(hit,r)|(hit==crate::settings_state::PickerHit::Edit(target)).then_some(r)),
    }
}
pub fn controls(app:&App)->Vec<(Hit,R)> {
    app.settings_ui.editor.as_ref().and_then(|e|target_rect(app,e.target))
        .map_or_else(Vec::new,|r|vec![(Hit::Field,r)])
}
pub fn message(app:&App,target:Target)->Option<String> {
    let editor=app.settings_ui.editor.as_ref().filter(|e|e.target==target)?;
    if !editor.error.is_empty(){return Some(editor.error.clone());}
    let s=spec(app,target);
    Some(if target==Target::Hex {"6 位 HEX · Enter 确认，Esc 取消".into()}
        else {format!("{}–{} {} · Enter 确认，Esc 取消",s.min,s.max,s.unit)})
}

/// Values remain plain labels. Editing adds selection and a caret in the same place.
pub fn draw_value(app:&App,fonts:&Fonts,ctx:&mut Ctx,target:Target,r:R,px:f32,prefix:&str,suffix:&str) {
    let palette=ctx.palette;
    let editor=app.settings_ui.editor.as_ref().filter(|e|e.target==target);
    let normal=value_text(app,target);
    let source=editor.map_or(normal.as_str(),|e|e.text.as_str());
    let before=fonts.text_width(prefix,px);let after=fonts.text_width(suffix,px);
    let available=(r.w as f32-before-after-10.0).max(10.0);
    let cursor=editor.map_or(source.len(),|e|e.cursor);
    let mut start=0;
    if editor.is_some_and(|e|!e.selected) {
        while start<cursor && fonts.text_width(&source[start..cursor],px)>available {start+=1;}
    }
    let mut end=start;
    while end<source.len() && fonts.text_width(&source[start..end+1],px)<=available {end+=1;}
    let shown=&source[start..end];let width=fonts.text_width(shown,px);
    let x=r.x as f32+(r.w as f32-before-width-after)/2.0;
    let baseline=r.y as f32+(r.h as f32+px*0.72)/2.0;
    let value_x=x+before;
    let ink=if target==Target::DotSize&&!app.whiteboard_dots {palette.disabled_ink}else{palette.text};
    ctx.text(fonts,prefix,x,baseline,px,c(palette.muted));
    if let Some(editor)=editor {
        if editor.selected {
            ctx.fill_round(R {x:value_x.floor() as i32-2,y:(baseline-px*0.82) as i32,
                w:width.ceil() as i32+4,h:(px+3.0) as i32},2.0,c(palette.accent_light));
        } else {
            let caret=value_x+fonts.text_width(&source[start..cursor.min(end)],px);
            ctx.stroke_line((caret,baseline-px*0.82),(caret,baseline+2.0),c(palette.text),1.0);
        }
        ctx.stroke_line((value_x-2.0,baseline+5.0),(value_x+width.max(12.0)+2.0,baseline+5.0),
            c(if editor.error.is_empty(){palette.accent}else{palette.danger}),1.0);
    }
    ctx.text(fonts,shown,value_x,baseline,px,c(ink));
    ctx.text(fonts,suffix,value_x+width,baseline,px,c(palette.muted));
}

pub fn draw(app:&App,fonts:&Fonts,ctx:&mut Ctx) {
    // Settings and the colour/page pickers show validation in their existing captions.
    let Some(editor)=app.settings_ui.editor.as_ref().filter(|e|e.target==Target::DotSize&&!e.error.is_empty()) else{return};
    let bar=ui::whiteboard_bar_rect(app);
    let r=R {x:bar.x,y:bar.y-26,w:bar.w,h:24};
    ctx.fill_round(r,4.0,c(ctx.palette.paper));
    ctx.text_center(fonts,r,&crate::settings_panel::fitted(fonts,&editor.error,12.0,r.w-16),12.0,c(ctx.palette.danger));
}
