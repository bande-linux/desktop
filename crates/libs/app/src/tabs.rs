use bande_toolkit::{DragAction, DragEvent, Point, Window, WindowDrag};

use crate::{Cx, Error, Program, View, WindowId};

pub trait TabHost: View {
    type Tab;

    fn release_tab(&mut self, window: &mut Window, id: u64) -> Option<Self::Tab>;

    fn adopt_tab(&mut self, window: &mut Window, tab: Self::Tab, index: usize);

    fn slide_tab_to(&mut self, window: &mut Window, id: u64, x: f32);

    fn insertion_index(&self, x: f32) -> usize;

    fn grab_point(&self, grab: Point) -> Point;
}

pub struct Detached<T> {
    pub tab: T,
    pub id: u64,
    pub index: usize,
    pub drag: WindowDrag,
    pub grab: Point,
    pub size: (u32, u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabDrop {
    Hover(f32),
    Unhover,
    Drop,
    Ended { placed: bool },
}

#[derive(Default)]
pub struct TabDropTarget {
    over: bool,
}

impl TabDropTarget {
    pub fn event(
        &mut self,
        window: &mut Window,
        strip_height: Option<f32>,
        drag: DragEvent,
    ) -> Option<TabDrop> {
        match drag {
            DragEvent::Entered { position, .. } | DragEvent::Moved(position) => {
                let over = strip_height.is_some_and(|height| (0.0..height).contains(&position.y));
                window.accept_drag_only(over.then_some(DragAction::Move));
                let was = std::mem::replace(&mut self.over, over);
                match (over, was) {
                    (true, _) => Some(TabDrop::Hover(position.x)),
                    (false, true) => Some(TabDrop::Unhover),
                    (false, false) => None,
                }
            }
            DragEvent::Left => std::mem::take(&mut self.over).then_some(TabDrop::Unhover),
            DragEvent::Dropped { .. } => std::mem::take(&mut self.over).then_some(TabDrop::Drop),
            DragEvent::Finished(action) => Some(TabDrop::Ended {
                placed: action.is_some(),
            }),
        }
    }
}

struct Carried {
    from: WindowId,
    index: usize,
    id: u64,
    holder: WindowId,
    floating: bool,
    drag: WindowDrag,
    grab: Point,
    size: (u32, u32),
}

#[derive(Default)]
pub struct TabCarry {
    carried: Option<Carried>,
}

type Open<'a, P> = &'a mut dyn FnMut(&mut Cx<P>, (u32, u32)) -> Result<WindowId, Error>;

impl TabCarry {
    pub fn is_carrying(&self) -> bool {
        self.carried.is_some()
    }

    pub fn drop_event<P>(
        &mut self,
        cx: &mut Cx<P>,
        window: WindowId,
        drop: TabDrop,
        open: Open<'_, P>,
    ) where
        P: Program,
        P::View: TabHost,
    {
        match drop {
            TabDrop::Hover(x) => self.hover(cx, window, x),
            TabDrop::Unhover => self.unhover(cx, window, open),
            TabDrop::Drop => self.end(cx, true),
            TabDrop::Ended { placed } => self.end(cx, placed),
        }
    }

    pub fn detach<P>(
        &mut self,
        cx: &mut Cx<P>,
        from: WindowId,
        detached: Detached<<P::View as TabHost>::Tab>,
        open: Open<'_, P>,
    ) where
        P: Program,
        P::View: TabHost,
    {
        let Detached {
            tab,
            id,
            index,
            drag,
            grab,
            size,
        } = detached;
        match float(cx, tab, &drag, grab, size, open) {
            Ok(holder) => {
                self.carried = Some(Carried {
                    from,
                    index,
                    id,
                    holder,
                    floating: true,
                    drag,
                    grab,
                    size,
                });
            }
            Err(tab) => {
                if let Some((shown, view)) = cx.window(from) {
                    view.adopt_tab(shown, tab, index);
                }
            }
        }
    }

    fn hover<P>(&mut self, cx: &mut Cx<P>, over: WindowId, x: f32)
    where
        P: Program,
        P::View: TabHost,
    {
        let Some(carried) = &mut self.carried else {
            return;
        };
        if carried.holder == over {
            if let Some((shown, view)) = cx.window(over) {
                view.slide_tab_to(shown, carried.id, x);
            }
            return;
        }
        if !carried.floating {
            return;
        }
        let Some(index) = cx.view(over).map(|view| view.insertion_index(x)) else {
            return;
        };
        let Some(tab) = cx
            .window(carried.holder)
            .and_then(|(shown, view)| view.release_tab(shown, carried.id))
        else {
            return;
        };
        cx.close(carried.holder);
        if let Some((shown, view)) = cx.window(over) {
            view.adopt_tab(shown, tab, index);
        }
        carried.holder = over;
        carried.floating = false;
    }

    fn unhover<P>(&mut self, cx: &mut Cx<P>, from: WindowId, open: Open<'_, P>)
    where
        P: Program,
        P::View: TabHost,
    {
        let Some(carried) = self.carried.take() else {
            return;
        };
        if carried.floating || carried.holder != from {
            self.carried = Some(carried);
            return;
        }
        let Some(tab) = cx
            .window(from)
            .and_then(|(shown, view)| view.release_tab(shown, carried.id))
        else {
            self.carried = Some(carried);
            return;
        };
        match float(cx, tab, &carried.drag, carried.grab, carried.size, open) {
            Ok(holder) => {
                self.carried = Some(Carried {
                    holder,
                    floating: true,
                    ..carried
                });
            }
            Err(tab) => {
                if let Some((shown, view)) = cx.window(from) {
                    view.adopt_tab(shown, tab, 0);
                }
                self.carried = Some(carried);
            }
        }
    }

    fn end<P>(&mut self, cx: &mut Cx<P>, placed: bool)
    where
        P: Program,
        P::View: TabHost,
    {
        let Some(carried) = self.carried.take() else {
            return;
        };
        if placed {
            return cx.raise(carried.holder, None);
        }
        if carried.holder != carried.from
            && let Some(tab) = cx
                .window(carried.holder)
                .and_then(|(shown, view)| view.release_tab(shown, carried.id))
        {
            if carried.floating {
                cx.close(carried.holder);
            }
            if let Some((shown, view)) = cx.window(carried.from) {
                view.adopt_tab(shown, tab, carried.index);
            }
        }
        cx.raise(carried.from, None);
    }
}

fn float<P>(
    cx: &mut Cx<P>,
    tab: <P::View as TabHost>::Tab,
    drag: &WindowDrag,
    grab: Point,
    size: (u32, u32),
    open: Open<'_, P>,
) -> Result<WindowId, <P::View as TabHost>::Tab>
where
    P: Program,
    P::View: TabHost,
{
    let window = match open(cx, size) {
        Ok(window) => window,
        Err(error) => {
            tracing::warn!("cannot open a window for the tab: {error}");
            return Err(tab);
        }
    };
    if let Some((shown, view)) = cx.window(window) {
        view.adopt_tab(shown, tab, 0);
        if let Err(error) = shown.attach_to_drag(drag, view.grab_point(grab)) {
            tracing::warn!("cannot carry the window with the tab: {error}");
        }
    }
    Ok(window)
}
