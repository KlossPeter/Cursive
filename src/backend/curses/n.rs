extern crate ncurses;

use self::super::find_closest;
use backend;
use std::cell::{RefCell, Cell};
use std::collections::HashMap;
use event::{Event, Key, MouseEvent, MouseButton};
use theme::{Color, ColorStyle, Effect};
use utf8;
use vec::Vec2;

pub struct Concrete {
    event_queue: Vec<Event>,
    current_style: Cell<ColorPair>,
    pairs: RefCell<HashMap<ColorPair, i16>>,
}

impl Concrete {
    /// Save a new color pair.
    fn insert_color(&self, pairs: &mut HashMap<ColorPair, i16>,
                    pair: ColorPair)
                    -> i16 {

        let n = 1 + pairs.len() as i16;
        let target = if ncurses::COLOR_PAIRS() > n as i32 {
            // We still have plenty of space for everyone.
            n
        } else {
            // The world is too small for both of us.
            let target = n - 1;
            // Remove the mapping to n-1
            pairs.retain(|_, &mut v| v != target);
            target
        };
        pairs.insert(pair, target);
        ncurses::init_pair(target,
                           find_closest(&pair.front),
                           find_closest(&pair.back));
        target
    }

    /// Checks the pair in the cache, or re-define a color if needed.
    fn get_or_create(&self, pair: ColorPair) -> i16 {

        let mut pairs = self.pairs.borrow_mut();

        // Find if we have this color in stock
        if pairs.contains_key(&pair) {
            // We got it!
            pairs[&pair]
        } else {
            self.insert_color(&mut *pairs, pair)
        }
    }

    fn set_colors(&self, pair: ColorPair) {

        let i = self.get_or_create(pair);

        self.current_style.set(pair);
        let style = ncurses::COLOR_PAIR(i);
        ncurses::attron(style);
    }
}

impl backend::Backend for Concrete {
    fn init() -> Self {
        // The delay is the time ncurses wait after pressing ESC
        // to see if it's an escape sequence.
        // Default delay is way too long. 25 is imperceptible yet works fine.
        ncurses::setlocale(ncurses::LcCategory::all, "");
        ::std::env::set_var("ESCDELAY", "25");
        ncurses::initscr();
        ncurses::keypad(ncurses::stdscr(), true);
        ncurses::mousemask(ncurses::ALL_MOUSE_EVENTS as ncurses::mmask_t,
                           None);
        ncurses::noecho();
        ncurses::cbreak();
        ncurses::start_color();
        ncurses::use_default_colors();
        ncurses::curs_set(ncurses::CURSOR_VISIBILITY::CURSOR_INVISIBLE);

        Concrete {
            current_style: Cell::new(ColorPair::from_256colors(0, 0)),
            pairs: RefCell::new(HashMap::new()),
            event_queue: Vec::new(),
        }
    }

    fn screen_size(&self) -> (usize, usize) {
        let mut x: i32 = 0;
        let mut y: i32 = 0;
        ncurses::getmaxyx(ncurses::stdscr(), &mut y, &mut x);
        (x as usize, y as usize)
    }

    fn has_colors(&self) -> bool {
        ncurses::has_colors()
    }

    fn finish(&mut self) {
        ncurses::endwin();
    }


    fn with_color<F: FnOnce()>(&self, colors: ColorPair, f: F) {
        let current = self.current_style.get();
        if current != colors {
            self.set_colors(colors);
        }

        f();

        if current != colors {
            self.set_colors(current);
        }
    }

    fn with_effect<F: FnOnce()>(&self, effect: Effect, f: F) {
        let style = match effect {
            Effect::Reverse => ncurses::A_REVERSE(),
            Effect::Simple => ncurses::A_NORMAL(),
        };
        ncurses::attron(style);
        f();
        ncurses::attroff(style);
    }

    fn clear(&self, color: Color) {
        let id = self.get_or_create(ColorPair {
                                        front: color,
                                        back: color,
                                    });
        ncurses::wbkgd(ncurses::stdscr(), ncurses::COLOR_PAIR(id));

        ncurses::clear();
    }

    fn refresh(&mut self) {
        ncurses::refresh();
    }

    fn print_at(&self, (x, y): (usize, usize), text: &str) {
        ncurses::mvaddstr(y as i32, x as i32, text);
    }

    fn poll_event(&mut self) -> Event {
        if !self.event_queue.is_empty() {
            return self.event_queue.remove(0);
        }
        let ch: i32 = ncurses::getch();

        // Is it a UTF-8 starting point?
        if 32 <= ch && ch <= 255 && ch != 127 {
            Event::Char(utf8::read_char(ch as u8,
                                        || Some(ncurses::getch() as u8))
                                .unwrap())
        } else {
            self.parse_ncurses_char(ch)
        }
    }

    fn set_refresh_rate(&mut self, fps: u32) {
        if fps == 0 {
            ncurses::timeout(-1);
        } else {
            ncurses::timeout(1000 / fps as i32);
        }
    }
}

impl Concrete {
    /// Returns the Key enum corresponding to the given ncurses event.
    fn parse_ncurses_char(&mut self, ch: i32) -> Event {
        match ch {
            // Value sent by ncurses when nothing happens
            -1 => Event::Refresh,

            // Values under 256 are chars and control values
            //
            // Tab is '\t'
            9 => Event::Key(Key::Tab),
            // Treat '\n' and the numpad Enter the same
            10 |
            13 |
            ncurses::KEY_ENTER => Event::Key(Key::Enter),
            // This is the escape key when pressed by itself.
            // When used for control sequences, it should have been caught earlier.
            27 => Event::Key(Key::Esc),
            // `Backspace` sends 127, but Ctrl-H sends `Backspace`
            127 |
            ncurses::KEY_BACKSPACE => Event::Key(Key::Backspace),

            409 => {
                let mut mevent = ncurses::MEVENT {
                    x: 0,
                    y: 0,
                    z: 0,
                    id: 0,
                    bstate: 0,
                };
                ncurses::getmouse(&mut mevent as *mut ncurses::MEVENT);
                let pos = Vec2::new(mevent.x as usize, mevent.y as usize);
                let bstate = mevent.bstate as i32;

                let modifier_mask = ncurses::BUTTON_ALT |
                                    ncurses::BUTTON_SHIFT |
                                    ncurses::BUTTON_CTRL;

                let ctrl = bstate & ncurses::BUTTON_CTRL != 0;
                let alt = bstate & ncurses::BUTTON_ALT != 0;
                let shift = bstate & ncurses::BUTTON_SHIFT != 0;

                let bstate = bstate & !modifier_mask;


                self.parse_mouse_button(bstate,
                                        |event| match (ctrl, alt, shift) {
                                            (false, false, false) => {
                                                Event::Mouse { pos, event }
                                            }
                                            (true, false, false) => {
                                                Event::CtrlMouse { pos, event }
                                            }
                                            (true, true, false) => {
                                                Event::CtrlAltMouse { pos, event }
                                            }
                                            (false, true, false) => {
                                                Event::AltMouse { pos, event }
                                            }
                                            (false, true, true) => {
                                                Event::AltShiftMouse { pos, event }
                                            }
                                            (true, false, true) => Event::CtrlShiftMouse{pos, event},
                                            (false, false, true) => Event::ShiftMouse{pos, event},
                                            (true, true, true) => unreachable!(),
                                        })
            }
            410 => Event::WindowResize,

            // Values 512 and above are probably extensions
            // Those keys don't seem to be documented...
            519 => Event::Alt(Key::Del),
            520 => Event::AltShift(Key::Del),
            521 => Event::Ctrl(Key::Del),
            522 => Event::CtrlShift(Key::Del),
            // 523: CtrlAltDel?
            //
            // 524?
            525 => Event::Alt(Key::Down),
            526 => Event::AltShift(Key::Down),
            527 => Event::Ctrl(Key::Down),
            528 => Event::CtrlShift(Key::Down),
            529 => Event::CtrlAlt(Key::Down),

            530 => Event::Alt(Key::End),
            531 => Event::AltShift(Key::End),
            532 => Event::Ctrl(Key::End),
            533 => Event::CtrlShift(Key::End),
            534 => Event::CtrlAlt(Key::End),

            535 => Event::Alt(Key::Home),
            536 => Event::AltShift(Key::Home),
            537 => Event::Ctrl(Key::Home),
            538 => Event::CtrlShift(Key::Home),
            539 => Event::CtrlAlt(Key::Home),

            540 => Event::Alt(Key::Ins),
            // 541: AltShiftIns?
            542 => Event::Ctrl(Key::Ins),
            // 543: CtrlShiftIns?
            544 => Event::CtrlAlt(Key::Ins),

            545 => Event::Alt(Key::Left),
            546 => Event::AltShift(Key::Left),
            547 => Event::Ctrl(Key::Left),
            548 => Event::CtrlShift(Key::Left),
            549 => Event::CtrlAlt(Key::Left),

            550 => Event::Alt(Key::PageDown),
            551 => Event::AltShift(Key::PageDown),
            552 => Event::Ctrl(Key::PageDown),
            553 => Event::CtrlShift(Key::PageDown),
            554 => Event::CtrlAlt(Key::PageDown),

            555 => Event::Alt(Key::PageUp),
            556 => Event::AltShift(Key::PageUp),
            557 => Event::Ctrl(Key::PageUp),
            558 => Event::CtrlShift(Key::PageUp),
            559 => Event::CtrlAlt(Key::PageUp),

            560 => Event::Alt(Key::Right),
            561 => Event::AltShift(Key::Right),
            562 => Event::Ctrl(Key::Right),
            563 => Event::CtrlShift(Key::Right),
            564 => Event::CtrlAlt(Key::Right),
            // 565?
            566 => Event::Alt(Key::Up),
            567 => Event::AltShift(Key::Up),
            568 => Event::Ctrl(Key::Up),
            569 => Event::CtrlShift(Key::Up),
            570 => Event::CtrlAlt(Key::Up),

            ncurses::KEY_B2 => Event::Key(Key::NumpadCenter),
            ncurses::KEY_DC => Event::Key(Key::Del),
            ncurses::KEY_IC => Event::Key(Key::Ins),
            ncurses::KEY_BTAB => Event::Shift(Key::Tab),
            ncurses::KEY_SLEFT => Event::Shift(Key::Left),
            ncurses::KEY_SRIGHT => Event::Shift(Key::Right),
            ncurses::KEY_LEFT => Event::Key(Key::Left),
            ncurses::KEY_RIGHT => Event::Key(Key::Right),
            ncurses::KEY_UP => Event::Key(Key::Up),
            ncurses::KEY_DOWN => Event::Key(Key::Down),
            ncurses::KEY_SR => Event::Shift(Key::Up),
            ncurses::KEY_SF => Event::Shift(Key::Down),
            ncurses::KEY_PPAGE => Event::Key(Key::PageUp),
            ncurses::KEY_NPAGE => Event::Key(Key::PageDown),
            ncurses::KEY_HOME => Event::Key(Key::Home),
            ncurses::KEY_END => Event::Key(Key::End),
            ncurses::KEY_SHOME => Event::Shift(Key::Home),
            ncurses::KEY_SEND => Event::Shift(Key::End),
            ncurses::KEY_SDC => Event::Shift(Key::Del),
            ncurses::KEY_SNEXT => Event::Shift(Key::PageDown),
            ncurses::KEY_SPREVIOUS => Event::Shift(Key::PageUp),
            // All Fn keys use the same enum with associated number
            f @ ncurses::KEY_F1...ncurses::KEY_F12 => {
                Event::Key(Key::from_f((f - ncurses::KEY_F0) as u8))
            }
            f @ 277...288 => Event::Shift(Key::from_f((f - 276) as u8)),
            f @ 289...300 => Event::Ctrl(Key::from_f((f - 288) as u8)),
            f @ 301...312 => Event::CtrlShift(Key::from_f((f - 300) as u8)),
            f @ 313...324 => Event::Alt(Key::from_f((f - 312) as u8)),
            // Values 8-10 (H,I,J) are used by other commands,
            // so we probably won't receive them. Meh~
            c @ 1...25 => Event::CtrlChar((b'a' + (c - 1) as u8) as char),
            other => {
                // Split the i32 into 4 bytes
                Event::Unknown(get_bytes(other))
            }
        }
    }

    fn parse_mouse_button<F>(&mut self, bstate: i32, wrapper: F) -> Event
        where F: Fn(MouseEvent) -> Event
    {
        let button = match bstate {
            ncurses::BUTTON1_RELEASED |
            ncurses::BUTTON1_PRESSED |
            ncurses::BUTTON1_CLICKED |
            ncurses::BUTTON1_DOUBLE_CLICKED |
            ncurses::BUTTON1_TRIPLE_CLICKED => Some(MouseButton::Left),
            ncurses::BUTTON2_RELEASED |
            ncurses::BUTTON2_PRESSED |
            ncurses::BUTTON2_CLICKED |
            ncurses::BUTTON2_DOUBLE_CLICKED |
            ncurses::BUTTON2_TRIPLE_CLICKED => Some(MouseButton::Middle),
            ncurses::BUTTON3_RELEASED |
            ncurses::BUTTON3_PRESSED |
            ncurses::BUTTON3_CLICKED |
            ncurses::BUTTON3_DOUBLE_CLICKED |
            ncurses::BUTTON3_TRIPLE_CLICKED => Some(MouseButton::Right),
            _ => None,
        };

        wrapper(match bstate {
                    ncurses::BUTTON1_RELEASED |
                    ncurses::BUTTON2_RELEASED |
                    ncurses::BUTTON3_RELEASED => {
                        MouseEvent::Release(button.unwrap())
                    }
                    ncurses::BUTTON1_PRESSED |
                    ncurses::BUTTON2_PRESSED |
                    ncurses::BUTTON3_PRESSED => {
                        MouseEvent::Press(button.unwrap())
                    }
                    ncurses::BUTTON1_CLICKED |
                    ncurses::BUTTON2_CLICKED |
                    ncurses::BUTTON3_CLICKED |
                    ncurses::BUTTON1_DOUBLE_CLICKED |
                    ncurses::BUTTON2_DOUBLE_CLICKED |
                    ncurses::BUTTON3_DOUBLE_CLICKED |
                    ncurses::BUTTON1_TRIPLE_CLICKED |
                    ncurses::BUTTON2_TRIPLE_CLICKED |
                    ncurses::BUTTON3_TRIPLE_CLICKED => {
            self.event_queue
                .push(wrapper(MouseEvent::Release(button.unwrap())));
            MouseEvent::Press(button.unwrap())
        }
                    ncurses::BUTTON4_PRESSED => MouseEvent::WheelUp,
                    ncurses::BUTTON5_PRESSED => MouseEvent::WheelDown,
                    _ => return Event::Unknown(get_bytes(bstate)),
                })
    }
}


fn get_bytes(b: i32) -> Vec<u8> {
    (0..4).map(|i| ((b >> (8 * i)) & 0xFF) as u8).collect()
}
