use std::marker::PhantomData;

use wasm_bindgen::{JsCast, UnwrapThrowExt};
use xilem_core::{Id, MessageResult, VecSplice};

use crate::{
    interfaces::sealed::Sealed, vecmap::VecMap, view::DomNode, AttributeValue, ChangeFlags, Cx,
    Pod, View, ViewMarker, ViewSequence, HTML_NS, MATHML_NS, SVG_NS,
};

use super::interfaces::Element;

type CowStr = std::borrow::Cow<'static, str>;

/// The state associated with a HTML element `View`.
///
/// Stores handles to the child elements and any child state, as well as attributes and event listeners
pub struct ElementState<ViewSeqState> {
    pub(crate) children_states: ViewSeqState,
    pub(crate) attributes: VecMap<CowStr, AttributeValue>,
    pub(crate) child_elements: Vec<Pod>,
    pub(crate) scratch: Vec<Pod>,
}

// TODO something like the `after_update` of the former `Element` view (likely as a wrapper view instead)

pub struct CustomElement<T, A = (), Children = ()> {
    name: CowStr,
    children: Children,
    #[allow(clippy::type_complexity)]
    phantom: PhantomData<fn() -> (T, A)>,
}

/// Builder function for a custom element view.
pub fn custom_element<T, A, Children: ViewSequence<T, A>>(
    name: impl Into<CowStr>,
    children: Children,
) -> CustomElement<T, A, Children> {
    CustomElement {
        name: name.into(),
        children,
        phantom: PhantomData,
    }
}

impl<T, A, Children> CustomElement<T, A, Children> {
    fn node_name(&self) -> &str {
        &self.name
    }
}

impl<T, A, Children> ViewMarker for CustomElement<T, A, Children> {}
impl<T, A, Children> Sealed for CustomElement<T, A, Children> {}

impl<T, A, Children> View<T, A> for CustomElement<T, A, Children>
where
    Children: ViewSequence<T, A>,
{
    type State = ElementState<Children::State>;

    // This is mostly intended for Autonomous custom elements,
    // TODO: Custom builtin components need some special handling (`document.createElement("p", { is: "custom-component" })`)
    type Element = web_sys::HtmlElement;

    fn build(&self, cx: &mut Cx) -> (Id, Self::State, Self::Element) {
        let (el, attributes) = cx.build_element(HTML_NS, &self.name);

        let mut child_elements = vec![];
        let (id, children_states) =
            cx.with_new_id(|cx| self.children.build(cx, &mut child_elements));

        for child in &child_elements {
            el.append_child(child.0.as_node_ref()).unwrap_throw();
        }

        // Set the id used internally to the `data-debugid` attribute.
        // This allows the user to see if an element has been re-created or only altered.
        #[cfg(debug_assertions)]
        el.set_attribute("data-debugid", &id.to_raw().to_string())
            .unwrap_throw();

        let el = el.dyn_into().unwrap_throw();
        let state = ElementState {
            children_states,
            child_elements,
            scratch: vec![],
            attributes,
        };
        (id, state, el)
    }

    fn rebuild(
        &self,
        cx: &mut Cx,
        prev: &Self,
        id: &mut Id,
        state: &mut Self::State,
        element: &mut Self::Element,
    ) -> ChangeFlags {
        let mut changed = ChangeFlags::empty();

        // update tag name
        if prev.name != self.name {
            // recreate element
            let parent = element
                .parent_element()
                .expect_throw("this element was mounted and so should have a parent");
            parent.remove_child(element).unwrap_throw();
            let (new_element, attributes) = cx.build_element(HTML_NS, self.node_name());
            state.attributes = attributes;
            // TODO could this be combined with child updates?
            while element.child_element_count() > 0 {
                new_element
                    .append_child(&element.child_nodes().get(0).unwrap_throw())
                    .unwrap_throw();
            }
            *element = new_element.dyn_into().unwrap_throw();
            changed |= ChangeFlags::STRUCTURE;
        }

        changed |= cx.rebuild_element(element, &mut state.attributes);

        // update children
        let mut splice = VecSplice::new(&mut state.child_elements, &mut state.scratch);
        changed |= cx.with_id(*id, |cx| {
            self.children
                .rebuild(cx, &prev.children, &mut state.children_states, &mut splice)
        });
        if changed.contains(ChangeFlags::STRUCTURE) {
            // This is crude and will result in more DOM traffic than needed.
            // The right thing to do is diff the new state of the children id
            // vector against the old, and derive DOM mutations from that.
            while let Some(child) = element.first_child() {
                element.remove_child(&child).unwrap_throw();
            }
            for child in &state.child_elements {
                element.append_child(child.0.as_node_ref()).unwrap_throw();
            }
            changed.remove(ChangeFlags::STRUCTURE);
        }
        changed
    }

    fn message(
        &self,
        id_path: &[Id],
        state: &mut Self::State,
        message: Box<dyn std::any::Any>,
        app_state: &mut T,
    ) -> MessageResult<A> {
        self.children
            .message(id_path, &mut state.children_states, message, app_state)
    }
}

impl<T, A, Children: ViewSequence<T, A>> Element<T, A> for CustomElement<T, A, Children> {}
impl<T, A, Children: ViewSequence<T, A>> crate::interfaces::HtmlElement<T, A>
    for CustomElement<T, A, Children>
{
}

macro_rules! generate_dom_interface_impl {
    ($dom_interface:ident, ($ty_name:ident, $t:ident, $a:ident, $vs:ident)) => {
        impl<$t, $a, $vs> $crate::interfaces::$dom_interface<$t, $a> for $ty_name<$t, $a, $vs> where
            $vs: $crate::view::ViewSequence<$t, $a>
        {
        }
    };
}

// TODO maybe it's possible to reduce even more in the impl function bodies and put into impl_functions
//      (should improve compile times and probably wasm binary size)
macro_rules! define_element {
    (($ns:expr, $ty_name:ident, $name:ident, $dom_interface:ident)) => {
        define_element!(($ns, $ty_name, $name, $dom_interface, T, A, VS));
    };
    (($ns:expr, $ty_name:ident, $name:ident, $dom_interface:ident, $t:ident, $a: ident, $vs: ident)) => {
        pub struct $ty_name<$t, $a = (), $vs = ()>($vs, PhantomData<fn() -> ($t, $a)>);

        impl<$t, $a, $vs> ViewMarker for $ty_name<$t, $a, $vs> {}
        impl<$t, $a, $vs> Sealed for $ty_name<$t, $a, $vs> {}

        impl<$t, $a, $vs: ViewSequence<$t, $a>> View<$t, $a> for $ty_name<$t, $a, $vs> {
            type State = ElementState<$vs::State>;
            type Element = web_sys::$dom_interface;

            fn build(&self, cx: &mut Cx) -> (Id, Self::State, Self::Element) {
                let (el, attributes) = cx.build_element($ns, stringify!($name));

                let mut child_elements = vec![];
                let (id, children_states) =
                    cx.with_new_id(|cx| self.0.build(cx, &mut child_elements));
                for child in &child_elements {
                    el.append_child(child.0.as_node_ref()).unwrap_throw();
                }

                // Set the id used internally to the `data-debugid` attribute.
                // This allows the user to see if an element has been re-created or only altered.
                #[cfg(debug_assertions)]
                el.set_attribute("data-debugid", &id.to_raw().to_string())
                    .unwrap_throw();

                let el = el.dyn_into().unwrap_throw();
                let state = ElementState {
                    children_states,
                    child_elements,
                    scratch: vec![],
                    attributes,
                };
                (id, state, el)
            }

            fn rebuild(
                &self,
                cx: &mut Cx,
                prev: &Self,
                id: &mut Id,
                state: &mut Self::State,
                element: &mut Self::Element,
            ) -> ChangeFlags {
                let mut changed = ChangeFlags::empty();

                changed |= cx.apply_attribute_changes(element, &mut state.attributes);

                // update children
                let mut splice = VecSplice::new(&mut state.child_elements, &mut state.scratch);
                changed |= cx.with_id(*id, |cx| {
                    self.0
                        .rebuild(cx, &prev.0, &mut state.children_states, &mut splice)
                });
                if changed.contains(ChangeFlags::STRUCTURE) {
                    // This is crude and will result in more DOM traffic than needed.
                    // The right thing to do is diff the new state of the children id
                    // vector against the old, and derive DOM mutations from that.
                    while let Some(child) = element.first_child() {
                        element.remove_child(&child).unwrap_throw();
                    }
                    for child in &state.child_elements {
                        element.append_child(child.0.as_node_ref()).unwrap_throw();
                    }
                    changed.remove(ChangeFlags::STRUCTURE);
                }
                changed
            }

            fn message(
                &self,
                id_path: &[Id],
                state: &mut Self::State,
                message: Box<dyn std::any::Any>,
                app_state: &mut $t,
            ) -> MessageResult<$a> {
                self.0
                    .message(id_path, &mut state.children_states, message, app_state)
            }
        }

        /// Builder function for a
        #[doc = concat!("`", stringify!($name), "`")]
        /// element view.
        pub fn $name<$t, $a, $vs: ViewSequence<$t, $a>>(children: $vs) -> $ty_name<$t, $a, $vs> {
            $ty_name(children, PhantomData)
        }

        generate_dom_interface_impl!($dom_interface, ($ty_name, $t, $a, $vs));

        paste::paste! {
            $crate::interfaces::[<for_all_ $dom_interface:snake _ancestors>]!(generate_dom_interface_impl, ($ty_name, $t, $a, $vs));
        }
    };
}

macro_rules! define_elements {
    ($($element_def:tt,)*) => {
        $(define_element!($element_def);)*
    };
}

define_elements!(
    // the order is copied from
    // https://developer.mozilla.org/en-US/docs/Web/HTML/Element
    // DOM interfaces copied from https://html.spec.whatwg.org/multipage/grouping-content.html and friends

    // TODO include document metadata elements?

    // content sectioning
    (HTML_NS, Address, address, HtmlElement),
    (HTML_NS, Article, article, HtmlElement),
    (HTML_NS, Aside, aside, HtmlElement),
    (HTML_NS, Footer, footer, HtmlElement),
    (HTML_NS, Header, header, HtmlElement),
    (HTML_NS, H1, h1, HtmlHeadingElement),
    (HTML_NS, H2, h2, HtmlHeadingElement),
    (HTML_NS, H3, h3, HtmlHeadingElement),
    (HTML_NS, H4, h4, HtmlHeadingElement),
    (HTML_NS, H5, h5, HtmlHeadingElement),
    (HTML_NS, H6, h6, HtmlHeadingElement),
    (HTML_NS, Hgroup, hgroup, HtmlElement),
    (HTML_NS, Main, main, HtmlElement),
    (HTML_NS, Nav, nav, HtmlElement),
    (HTML_NS, Section, section, HtmlElement),
    // text content
    (HTML_NS, Blockquote, blockquote, HtmlQuoteElement),
    (HTML_NS, Dd, dd, HtmlElement),
    (HTML_NS, Div, div, HtmlDivElement),
    (HTML_NS, Dl, dl, HtmlDListElement),
    (HTML_NS, Dt, dt, HtmlElement),
    (HTML_NS, Figcaption, figcaption, HtmlElement),
    (HTML_NS, Figure, figure, HtmlElement),
    (HTML_NS, Hr, hr, HtmlHrElement),
    (HTML_NS, Li, li, HtmlLiElement),
    (HTML_NS, Link, link, HtmlLinkElement),
    (HTML_NS, Menu, menu, HtmlMenuElement),
    (HTML_NS, Ol, ol, HtmlOListElement),
    (HTML_NS, P, p, HtmlParagraphElement),
    (HTML_NS, Pre, pre, HtmlPreElement),
    (HTML_NS, Ul, ul, HtmlUListElement),
    // inline text
    (HTML_NS, A, a, HtmlAnchorElement, T, A_, VS),
    (HTML_NS, Abbr, abbr, HtmlElement),
    (HTML_NS, B, b, HtmlElement),
    (HTML_NS, Bdi, bdi, HtmlElement),
    (HTML_NS, Bdo, bdo, HtmlElement),
    (HTML_NS, Br, br, HtmlBrElement),
    (HTML_NS, Cite, cite, HtmlElement),
    (HTML_NS, Code, code, HtmlElement),
    (HTML_NS, Data, data, HtmlDataElement),
    (HTML_NS, Dfn, dfn, HtmlElement),
    (HTML_NS, Em, em, HtmlElement),
    (HTML_NS, I, i, HtmlElement),
    (HTML_NS, Kbd, kbd, HtmlElement),
    (HTML_NS, Mark, mark, HtmlElement),
    (HTML_NS, Q, q, HtmlQuoteElement),
    (HTML_NS, Rp, rp, HtmlElement),
    (HTML_NS, Rt, rt, HtmlElement),
    (HTML_NS, Ruby, ruby, HtmlElement),
    (HTML_NS, S, s, HtmlElement),
    (HTML_NS, Samp, samp, HtmlElement),
    (HTML_NS, Small, small, HtmlElement),
    (HTML_NS, Span, span, HtmlSpanElement),
    (HTML_NS, Strong, strong, HtmlElement),
    (HTML_NS, Sub, sub, HtmlElement),
    (HTML_NS, Sup, sup, HtmlElement),
    (HTML_NS, Time, time, HtmlTimeElement),
    (HTML_NS, U, u, HtmlElement),
    (HTML_NS, Var, var, HtmlElement),
    (HTML_NS, Wbr, wbr, HtmlElement),
    // image and multimedia
    (HTML_NS, Area, area, HtmlAreaElement),
    (HTML_NS, Audio, audio, HtmlAudioElement),
    (HTML_NS, Canvas, canvas, HtmlCanvasElement),
    (HTML_NS, Img, img, HtmlImageElement),
    (HTML_NS, Map, map, HtmlMapElement),
    (HTML_NS, Track, track, HtmlTrackElement),
    (HTML_NS, Video, video, HtmlVideoElement),
    // embedded content
    (HTML_NS, Embed, embed, HtmlEmbedElement),
    (HTML_NS, Iframe, iframe, HtmlIFrameElement),
    (HTML_NS, Object, object, HtmlObjectElement),
    (HTML_NS, Picture, picture, HtmlPictureElement),
    (HTML_NS, Portal, portal, HtmlElement),
    (HTML_NS, Source, source, HtmlSourceElement),
    // scripting
    (HTML_NS, Noscript, noscript, HtmlElement),
    (HTML_NS, Script, script, HtmlScriptElement),
    // demarcating edits
    (HTML_NS, Del, del, HtmlModElement),
    (HTML_NS, Ins, ins, HtmlModElement),
    // tables
    (HTML_NS, Caption, caption, HtmlTableCaptionElement),
    (HTML_NS, Col, col, HtmlTableColElement),
    (HTML_NS, Colgroup, colgroup, HtmlTableColElement),
    (HTML_NS, Table, table, HtmlTableElement),
    (HTML_NS, Tbody, tbody, HtmlTableSectionElement),
    (HTML_NS, Td, td, HtmlTableCellElement),
    (HTML_NS, Tfoot, tfoot, HtmlTableSectionElement),
    (HTML_NS, Th, th, HtmlTableCellElement),
    (HTML_NS, Thead, thead, HtmlTableSectionElement),
    (HTML_NS, Tr, tr, HtmlTableRowElement),
    // forms
    (HTML_NS, Button, button, HtmlButtonElement),
    (HTML_NS, Datalist, datalist, HtmlDataListElement),
    (HTML_NS, Fieldset, fieldset, HtmlFieldSetElement),
    (HTML_NS, Form, form, HtmlFormElement),
    (HTML_NS, Input, input, HtmlInputElement),
    (HTML_NS, Label, label, HtmlLabelElement),
    (HTML_NS, Legend, legend, HtmlLegendElement),
    (HTML_NS, Meter, meter, HtmlMeterElement),
    (HTML_NS, Optgroup, optgroup, HtmlOptGroupElement),
    (HTML_NS, OptionElement, option, HtmlOptionElement), // Avoid cluttering the namespace with `Option`
    (HTML_NS, Output, output, HtmlOutputElement),
    (HTML_NS, Progress, progress, HtmlProgressElement),
    (HTML_NS, Select, select, HtmlSelectElement),
    (HTML_NS, Textarea, textarea, HtmlTextAreaElement),
    // interactive elements,
    (HTML_NS, Details, details, HtmlDetailsElement),
    (HTML_NS, Dialog, dialog, HtmlDialogElement),
    (HTML_NS, Summary, summary, HtmlElement),
    // web components,
    (HTML_NS, Slot, slot, HtmlSlotElement),
    (HTML_NS, Template, template, HtmlTemplateElement),
    // SVG and MathML (TODO, svg and mathml elements)
    (SVG_NS, Svg, svg, SvgElement),
    (MATHML_NS, Math, math, Element),
);