extern crate tcod;
extern crate bresenham;
extern crate rand;

use std::process::Command;

use tcod::console::*;
use tcod::colors;
use tcod::map::{Map as FovMap, FovAlgorithm};
use colors::Color;
use bresenham::Bresenham;
use rand::Rng;

// actual size of the window (in characters)
const SCREEN_WIDTH: i32 = 32;
const SCREEN_HEIGHT: i32 = 24;
const BAR_WIDTH: i32 = 16;
const PANEL_HEIGHT: i32 = 7;
const PANEL_Y: i32 = SCREEN_HEIGHT - PANEL_HEIGHT;
const MSG_X: i32 = 1;
const MSG_WIDTH: i32 = SCREEN_WIDTH - MSG_X - 1;
const MSG_HEIGHT: usize = PANEL_HEIGHT as usize - 1;

// size of map (in characters)
const MAP_WIDTH: i32 = 80 + SCREEN_WIDTH + 1;
const MAP_HEIGHT: i32 = 45 + SCREEN_HEIGHT + 1; 

const LIMIT_FPS: i32 = 20;  // 20 frames-per-second maximum

const COLOR_WALL_DARK: Color = colors::DARKEST_GREY;
const COLOR_WALL_LIGHT: Color = colors::DARKER_GREY;
const COLOR_GROUND_DARK: Color = colors::Color { r: 40, g: 5, b: 5 };
const COLOR_GROUND_LIGHT: Color = Color { r: 110, g: 10, b: 20 };
// const COLOR_WALL_DARK: Color = Color { r: 30, g: 5, b: 45 };
// const COLOR_WALL_LIGHT: Color = Color { r: 95, g: 20, b: 120 };
// const COLOR_GROUND_DARK: Color = colors:: DARKEST_GREY;
// const COLOR_GROUND_LIGHT: Color = colors:: DARK_GREY;

const FOV_ALGO: FovAlgorithm = FovAlgorithm::Shadow;
const FOV_LIGHT_WALLS: bool = true;
const TORCH_RADIUS: i32 = 7;

const ROOM_MAX_SIZE: i32 = 12;
const ROOM_MIN_SIZE: i32 = 6;
const MAX_ROOMS: i32 = 30;

const MAX_ROOM_MONSTERS: i32 = 3;
const MAX_ROOM_ITEMS: i32 = 1;

type Messages = Vec<(String, Color)>;

fn print_message<T: Into<String>>(messages: &mut Messages, message: T, color: Color) {
    if messages.len() == MSG_HEIGHT {
        messages.remove(0);
    }

    messages.push((message.into(), color));
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PlayerAction {
    TookTurn,
    DidntTakeTurn,
    Exit,
}

#[derive(Clone,Copy, Debug, PartialEq)]
enum DeathCallback {
    Player,
    Monster,
}

impl DeathCallback {
    fn callback(self, entity: &mut Entity, messages: &mut Messages) {
        use DeathCallback::*;
        let cb: fn(&mut Entity, &mut Messages) = match self {
            Player => player_death,
            Monster => monster_death,
        };
        cb(entity, messages);
    }
}

fn player_death(player: &mut Entity, messages: &mut Messages) {
    print_message(messages, "You died!", colors::RED);
    print_message(messages, "Press Start to start a new game!", colors::CYAN);
    player.char = '%';
    player.color = colors::DARK_RED;
    player.alive = false;
}

fn monster_death(monster: &mut Entity, messages: &mut Messages) {
    print_message(messages, format!("{} died!", monster.name), colors::RED);
    if monster.char == 'T' {
        monster.char = '%'
    } else {
        monster.char = '.';
    }
    monster.blocks = false;
    monster.ai = None;
    monster.fighter = None;
    monster.alive = false;
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Fighter {
    max_hp: i32,
    hp: i32,
    defense: i32,
    power: i32,
    on_death: DeathCallback,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Chest;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Item {
    Heal,
    Key
}

struct Inventory {
    healing_potions: i32,
    has_key: bool
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Ai;

impl Ai {
    pub fn take_turn(monster_id: usize, state: &mut GameState, messages: &mut Messages) {
        let monster = &mut state.npcs[monster_id];
        if state.fov_map.is_in_fov(monster.x, monster.y) {
            if !monster.next_to(&state.player) {
                monster.move_towards(state.player.x, state.player.y, &mut state.map);
            } else if state.player.fighter.map_or(false, |f| f.hp > 0) {
                monster.attack(&mut state.player, messages);
            }
        }
    }
}

#[derive(Debug)]
struct Entity {
    blocks: bool,
    x: i32,
    y: i32,
    char: char,
    color: Color,
    name: String,
    alive: bool,
    fighter: Option<Fighter>,
    ai: Option<Ai>,
    item: Option<Item>,
    chest: Option<Chest>,
}

impl Entity {
    pub fn new<S: Into<String>>(x: i32, y: i32, char: char, color: Color, name: S, map: &mut Map, blocks: bool, alive: bool) -> Option<Self> {
        let blocked = map.get(x, y).blocks_movement;
        if blocked {
            None
        } else {
            if blocks {
                map.set(x, y, Tile::entity());
            }
            Some(Entity {
                blocks,
                x,
                y,
                char,
                name: name.into(),
                color,
                alive,
                fighter: None,
                ai: None,
                item: None,
                chest: None,
            })
        }
    }

    pub fn move_towards(&mut self, target_x: i32, target_y: i32, map: &mut Map) {
        // vector from this object to the target, and distance
        let dx = target_x - self.x;
        let dy = target_y - self.y;
        let distance = ((dx.pow(2) + dy.pow(2)) as f32).sqrt();


        // normalize it to length 1 (preserving direction), then round it and
        // convert to integer so the movement is restricted to the map grid
        let mut dx = (dx as f32 / distance).round() as i32;
        let mut dy = (dy as f32 / distance).round() as i32;
        if dx.abs() > dy.abs() {
            dy = 0;
        } else {
            dx = 0;
        }
        self.move_by(dx, dy, map);
    }

    // pub fn distance_to(&self, other: &Entity) -> f32 {
    //     let dx = other.x - self.x;
    //     let dy = other.y - self.y;
    //     ((dx.pow(2) + dy.pow(2)) as f32).sqrt()
    // }

    pub fn next_to(&self, other: &Entity) -> bool {
        let dx = (other.x - self.x).abs();
        let dy = (other.y - self.y).abs();
        (dx == 0 && dy == 1) || (dy == 0 && dx == 1)
    }

    pub fn move_by(&mut self, dx: i32, dy: i32, map: &mut Map) {
        if !map.get(self.x + dx, self.y + dy).blocks_movement {
            if self.blocks {
                map.set(self.x, self.y, Tile::empty());
            }
            self.x += dx;
            self.y += dy;
            if self.blocks {
                map.set(self.x, self.y, Tile::entity());
            }
        }
    }

    pub fn move_or_attack(&mut self, dx: i32, dy: i32, map: &mut Map, enemies: &mut [Entity], objects: &mut [Entity], has_key: bool, stage: &mut GameStage, messages: &mut Messages) -> Option<usize> {
        if !map.get(self.x + dx, self.y + dy).blocks_movement {
            if self.blocks {
                map.set(self.x, self.y, Tile::empty());
            }
            self.x += dx;
            self.y += dy;
            if self.blocks {
                map.set(self.x, self.y, Tile::entity());
            }
        } else {
            let x = self.x + dx;
            let y = self.y + dy;
            let enemy_id = enemies.iter().position(|enemy| {
                enemy.x == x && enemy.y == y
            });

            match enemy_id {
                Some(id) => {
                    let enemy = &mut enemies[id];
                    self.attack(enemy, messages);
                    if !enemy.blocks {
                        map.set(enemy.x, enemy.y, Tile::empty());
                    }
                    if enemy.alive {
                        return Some(id);
                    } else {
                        return None;
                    }
                },
                None => {
                    let obj_id = objects.iter().position(|obj| {
                        obj.x == x && obj.y == y
                    });

                    match obj_id {
                        Some(id) => {
                            if objects[id].chest.is_some() {
                                if has_key {
                                    open_chest(messages, stage);
                                } else {
                                    print_message(messages, "You need a key to open this chest, not a sword.", colors::WHITE);
                                }
                            }
                        },
                        None => {
                            print_message(messages, "You try to attack... the wall?", colors::GREY);
                        }
                    }
                    return None;
                }
            }
        }
        None
    }

    pub fn take_damage(&mut self, damage: i32, messages: &mut Messages) {
        if let Some(f) = self.fighter.as_mut() {
            f.hp -= damage;
            f.hp = std::cmp::min(f.hp, f.max_hp);
        }
        if let Some(f) = self.fighter {
            if f.hp <= 0 {
                f.on_death.callback(self, messages);
            }
        }
    }

    pub fn attack(&mut self, target: &mut Entity, messages: &mut Messages) {
        let damage = self.fighter.map_or(0, |f| f.power) - target.fighter.map_or(0, |f| f.defense);
        if damage > 0 {
            let color = if self.name == "James" {
                colors::WHITE
            } else {
                colors::RED
            };
            print_message(messages, format!("{} attacks {} for {} hp!", self.name, target.name, damage), color);
            target.take_damage(damage, messages);
        } else {
            print_message(messages, format!("{} attacks {} but it has no effect... ", self.name, target.name), colors::GREY);
        }
    }

    pub fn draw(&self, con: &mut Console) {
        con.set_default_foreground(self.color);
        con.put_char(self.x, self.y, self.char, BackgroundFlag::None);
    }

    pub fn clear(&self, con: &mut Console) {
        con.put_char(self.x, self.y, ' ', BackgroundFlag::None);
    }
}

#[derive(Clone, Copy, Debug)]
struct Tile {
    blocks_movement: bool,
    blocks_sight: bool,
    explored: bool,
}

impl Tile {
    pub fn empty() -> Self {
        Tile { blocks_movement: false, blocks_sight: false, explored: false }
    }

    pub fn wall() -> Self {
        Tile { blocks_movement: true, blocks_sight: true, explored: false }
    }

    pub fn entity() -> Self {
        Tile { blocks_movement: true, blocks_sight: false, explored: false }
    }

    pub fn is_wall(&self) -> bool {
        self.blocks_movement && self.blocks_sight
    }
}

struct Map {
    width: i32,
    height: i32,
    data: Vec<Vec<Tile>>,
}

impl Map {
    pub fn new(width: i32, height: i32, default_tile: Tile) -> Self {
        Map {
            width,
            height,
            data: vec![vec![default_tile; height as usize]; width as usize]
        }
    }

    pub fn get(&self, x: i32, y: i32) -> Tile {
        self.data[x as usize][y as usize]
    }

    pub fn get_mut(&mut self, x: i32, y: i32) -> &mut Tile {
        &mut self.data[x as usize][y as usize]
    }

    pub fn set(&mut self, x: i32, y: i32, tile: Tile) {
        self.data[x as usize][y as usize] = tile
    } 

    pub fn draw(&self, con: &mut Console, player_pos: (i32, i32), fov_map: &FovMap) {
        for y in 0..(self.height - 1) {
            for x in 0..(self.width - 1) {
                if self.get(x, y).explored {
                    let visible = fov_map.is_in_fov(x, y);
                    let is_wall = self.get(x, y).is_wall();
                    let col = match (visible, is_wall) {
                        (false, true) => COLOR_WALL_DARK,
                        (false, false) => COLOR_GROUND_DARK,
                        (true, true) => colors::lerp(COLOR_WALL_LIGHT, COLOR_WALL_DARK, ((((x - player_pos.0).pow(2) + (y - player_pos.1).pow(2)) as f32).sqrt() / TORCH_RADIUS as f32).powi(2)),
                        (true, false) => colors::lerp(COLOR_GROUND_LIGHT, COLOR_GROUND_DARK, ((((x - player_pos.0).pow(2) + (y - player_pos.1).pow(2)) as f32).sqrt() / TORCH_RADIUS as f32).powi(2)),
                    };
                    if is_wall {
                        con.put_char_ex(x, y, '#',
                                        Color {
                                            r: std::cmp::max(col.r as i16 - 8, 0) as u8,
                                            g: std::cmp::max(col.g as i16 - 8, 0) as u8,
                                            b: std::cmp::max(col.b as i16 - 8, 0) as u8,
                                        }, 
                                        col);
                    } else {
                        con.set_char_background(x, y, col, BackgroundFlag::Set);
                    }
                }
            }
        }
    }

    pub fn clear(&self, con: &mut Console) {
        for y in 0..(self.height - 1) {
            for x in 0..(self.width - 1) {
                let is_wall = self.get(x, y).is_wall();
                if is_wall {
                    con.put_char(x, y, ' ', BackgroundFlag::None);
                }
            }
        }
    }

    pub fn set_rect(&mut self, rect: Rect, tile: Tile, inclusive: bool) {
        let initial_add = if inclusive { 0 } else { 1 };
        let after_add = if inclusive { 1 } else { 0 };
        for x in std::cmp::max(0, rect.x1 + initial_add)..std::cmp::min(self.width - 1, rect.x2 + after_add) {
            for y in std::cmp::max(0, rect.y1 + initial_add)..std::cmp::min(self.height - 1, rect.y2 + after_add) {
                self.set(x, y, tile);
            }
        }
    }

    pub fn set_tunnel(&mut self, start: (i32, i32), end: (i32, i32), radius: i32, tile: Tile) {
        for (x,y) in Bresenham::new((start.0 as isize, start.1 as isize), (end.0 as isize, end.1 as isize)) {
            self.set_rect(Rect::new(x as i32 - radius, y as i32 - radius, radius*2, radius*2), tile, true);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x1: i32,
    x2: i32,
    y1: i32,
    y2: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x1: x, y1: y, x2: x + w, y2: y + h }
    }

    pub fn center(&self) -> (i32, i32) {
        let center_x = (self.x1 + self.x2) / 2;
        let center_y = (self.y1 + self.y2) / 2;
        (center_x, center_y)
    }

    pub fn intersects_with(&self, other: &Rect) -> bool {
        // returns true if this rectangle intersects with another one
        (self.x1 <= other.x2) && (self.x2 >= other.x1) &&
            (self.y1 <= other.y2) && (self.y2 >= other.y1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum GameStage {
    Title,
    Playing,
    GameOver,
    Won,
}

struct GameState {
    player: Entity,
    npcs: Vec<Entity>,
    objects: Vec<Entity>,
    map: Map,
    camera_pos: (i32, i32),
    fov_map: FovMap,
    prev_player_pos: (i32, i32),
    stage: GameStage,
    recent_enemy_id: Option<usize>,
    inventory: Inventory,
}

fn open_chest(messages: &mut Messages, stage: &mut GameStage) {
    print_message(messages, "It sounds like the chest is opening... Congratulations, you win!", colors::CYAN);
    print_message(messages, "Press START to open your gift!", colors::LIGHT_MAGENTA);

    *stage = GameStage::Won;
    Command::new("sh")
        .arg("-c")
        .arg("/home/pi/clean_startup.sh")
        .spawn()
        .expect("failed to clean startup");
}

fn main() {
    let mut root = Root::initializer()
        .font("dejavu10x10_gs_tc.png", FontLayout::Tcod)
        .font_type(FontType::Greyscale)
        .size(SCREEN_WIDTH, SCREEN_HEIGHT)
        .title("Rust/libtcod tutorial")
        .init();

    let mut con = Offscreen::new(MAP_WIDTH, MAP_HEIGHT);

    let mut status = Offscreen::new(SCREEN_WIDTH, PANEL_HEIGHT);

    tcod::system::set_fps(LIMIT_FPS);
    tcod::input::show_cursor(false);


    let (initial_map, rooms, (px, py)) = generate_map(MAP_WIDTH-SCREEN_WIDTH-1, MAP_HEIGHT-SCREEN_HEIGHT-1);
    let (px, py) = (px + SCREEN_WIDTH/2, py + SCREEN_HEIGHT/2);

    let mut final_map = Map::new(MAP_WIDTH, MAP_HEIGHT, Tile::wall());

    for y in 0..MAP_HEIGHT-SCREEN_HEIGHT-1 {
        for x in 0..MAP_WIDTH-SCREEN_WIDTH-1 {
            final_map.set(x+SCREEN_WIDTH/2, y+SCREEN_HEIGHT/2, initial_map.get(x, y));
        }
    }

    let npcs = generate_monsters(&rooms[1..], &mut final_map);
    let objects = generate_objects(&rooms[..], &mut final_map);
    let mut player = Entity::new(px, py, '@', colors::WHITE, "James", &mut final_map, true, true).unwrap();
    player.fighter = Some(Fighter{
        max_hp: 30,
        hp: 30,
        defense: 2,
        power: 5,
        on_death: DeathCallback::Player,
    });

    let mut state = GameState {
        player,
        npcs,
        objects,
        map: final_map,
        camera_pos: (px, py),
        fov_map: FovMap::new(MAP_WIDTH, MAP_HEIGHT),
        prev_player_pos: (px, py),
        stage: GameStage::Title,
        recent_enemy_id: None,
        inventory: Inventory { healing_potions: 0, has_key: false },
    };

    let mut messages: Messages = vec![];

    print_message(&mut messages, "Hello James! Find the key in the Tomb of the Ancient King and bring it back here to unluck the box... or perish. Press Start to Begin!", colors::CYAN);

    // compute initial fov
    for y in 0..MAP_HEIGHT-1 {
        for x in 0..MAP_WIDTH {
            state.fov_map.set(x, y,
                !state.map.get(x, y).blocks_sight,
                !state.map.get(x, y).blocks_movement
            );
        }
    }
    compute_fov(&mut state, true);

    // Render initial state
    render_all(&mut root, &mut con, &mut status, &state, true, &mut messages);

    // Loop
    while !root.window_closed() {
        // handle keys and exit game if needed
        let action = handle_keys(&mut root, &mut con, &mut status, &mut state, &mut messages);
        match action {
            PlayerAction::Exit => break,
            PlayerAction::TookTurn => {
                handle_camera(&mut state);
                let fov_recmputed = compute_fov(&mut state, false);
                for id in 0..state.npcs.len() {
                    if state.npcs[id].ai.is_some() {
                        Ai::take_turn(id, &mut state, &mut messages);
                    }
                }
                let mut tbr = Vec::new();
                for (i, obj) in state.objects.iter().enumerate() {
                    if obj.x == state.player.x && obj.y == state.player.y {
                        match obj.item {
                            Some(item) => {
                                match item {
                                    Item::Heal => {
                                        state.inventory.healing_potions += 1;
                                        print_message(&mut messages, "You picked up a health potion!", colors::CHARTREUSE)
                                    },
                                    Item::Key => {
                                        state.inventory.has_key = true;
                                        print_message(&mut messages, "You picked up the key!", colors::CHARTREUSE)
                                    }
                                }
                                tbr.push(i);
                            },
                            None => {}
                        }
                    }
                }
                for i in tbr {
                    state.objects.remove(i);
                }
                render_all(&mut root, &mut con, &mut status, &mut state, fov_recmputed, &mut messages);
                state.prev_player_pos = (state.player.x, state.player.y);
            },
            PlayerAction::DidntTakeTurn => {}
        }
        if !state.player.alive {
            println!("Game over");
            state.stage = GameStage::GameOver;
        }
    }
}

fn generate_objects(rooms: &[Rect], map: &mut Map) -> Vec<Entity> {
    let mut objects = Vec::new();
    let start = rooms[0].center();
    let mut furthest_room: Rect = rooms[0];
    let mut furthest_dist = 0;
    let mut rng = rand::thread_rng();
    for room in rooms {
        let num_items = rand::thread_rng().gen_range(0, MAX_ROOM_ITEMS + 1);

        for _ in 0..num_items {
            // only place it if the tile is not blocked
            let mut i = 0;
            loop {
                let x = rng.gen_range(room.x1 + 1, room.x2);
                let y = rng.gen_range(room.y1 + 1, room.y2);
                let object = Entity::new(x + SCREEN_WIDTH / 2, y + SCREEN_HEIGHT / 2, '^', colors::LIGHT_CYAN, "healing potion", map, false, false);
                match object {
                    Some(mut m) => {
                        m.item = Some(Item::Heal);
                        objects.push(m);
                        break;
                    },
                    None => {
                        i += 1;
                        if i > 40 {
                            break;
                        }
                    },
                }
            }
        }

        let center = room.center();
        let dist = (center.0 - start.0).pow(2) + (center.1 - start.1).pow(2);
        if dist > furthest_dist {
            furthest_room = room.clone();
            furthest_dist = dist;
        }
    }

    let mut i = 0;
    loop {
        let x = rng.gen_range(furthest_room.x1 + 1, furthest_room.x2);
        let y = rng.gen_range(furthest_room.y1 + 1, furthest_room.y2);
        let object = Entity::new(x + SCREEN_WIDTH / 2, y + SCREEN_HEIGHT / 2, '!', colors::GOLD, "key", map, false, false);
        match object {
            Some(mut m) => {
                m.item = Some(Item::Key);
                objects.push(m);
                break;
            },
            None => {
                i += 1;
                if i > 40 {
                    break;
                }
            },
        }
    }
    let x = start.0;
    let y = start.1 - 1;
    let mut chest = Entity::new(x + SCREEN_WIDTH / 2, y + SCREEN_HEIGHT / 2, '&', colors::DARK_AMBER, "chest", map, true, false).unwrap();
    chest.chest = Some(Chest);
    objects.push(chest);

    objects
}

fn generate_monsters(rooms: &[Rect], map: &mut Map) -> Vec<Entity> {
    let mut npcs: Vec<Entity> = Vec::new();
    let mut rng = rand::thread_rng();
    for room in rooms {
        let mut num_monsters = rng.gen_range(0, MAX_ROOM_MONSTERS + 1);
        if num_monsters == 0 {
            if rng.gen() {
                num_monsters = 1;
            }
        }
        for _ in 0..num_monsters {
            loop {
                let x = rng.gen_range(room.x1 + 1, room.x2);
                let y = rng.gen_range(room.y1 + 1, room.y2);
                let monster = if rand::random::<f32>() < 0.8 {
                    Entity::new(x + SCREEN_WIDTH/2, y + SCREEN_HEIGHT/2, 'g', colors::DESATURATED_GREEN, "Goblin", map, true, true)
                } else {
                    Entity::new(x + SCREEN_WIDTH/2, y + SCREEN_HEIGHT/2, 'T', colors::DARK_GREEN, "Troll", map, true, true)
                };
                match monster {
                    Some(mut m) => {
                        if m.char == 'g' {
                            m.fighter = Some(Fighter{max_hp: 10, hp: 10, defense: 0, power: 3, on_death: DeathCallback::Monster});
                            m.ai = Some(Ai);
                        } else if m.char == 'T' {
                            m.fighter = Some(Fighter{max_hp: 16, hp: 16, defense: 1, power: 4, on_death: DeathCallback::Monster});
                            m.ai = Some(Ai);
                        }
                        npcs.push(m);
                        break;
                    },
                    None => continue,
                }
            }

        }
    }
    npcs
}

fn generate_map(width: i32, height: i32) -> (Map, Vec<Rect>, (i32, i32)) {
    let mut map: Map;
    let mut starting_position = (0, 0);
    let mut rooms: Vec<Rect>;
    loop {
        map = Map::new(width, height, Tile::wall());
        rooms = Vec::new();

        for _ in 0..MAX_ROOMS {
            let mut rng = rand::thread_rng();
            let w = rng.gen_range(ROOM_MIN_SIZE, ROOM_MAX_SIZE + 1);
            let h = rng.gen_range(ROOM_MIN_SIZE, ROOM_MAX_SIZE + 1);
            let x = rng.gen_range(0, map.width - w);
            let y = rng.gen_range(0, map.height - h);
            let new_room = Rect::new(x, y, w, h);

            let failed = rooms.iter().any(|other_room| new_room.intersects_with(other_room));
            if !failed {
                map.set_rect(new_room, Tile::empty(), false);
                let (nx, ny) = new_room.center();
                if rooms.is_empty() {
                    starting_position = (nx, ny);
                } else {
                    let (px, py) = rooms[rooms.len() - 1].center();

                    if rng.gen_range(0, 100) <= 10 {
                        map.set_tunnel((px, py), (nx, ny), 1, Tile::empty());
                    } else if rng.gen() {
                        map.set_rect(Rect{x1: std::cmp::min(px, nx), y1: py, x2: std::cmp::max(nx, px), y2: py}, Tile::empty(), true);
                        map.set_rect(Rect{x1: nx, y1: std::cmp::min(py, ny), x2: nx, y2: std::cmp::max(ny, py)}, Tile::empty(), true);
                    } else {
                        map.set_rect(Rect{x1: std::cmp::min(px, nx), y1: py, x2: std::cmp::max(nx, px), y2: py}, Tile::empty(), true);
                        map.set_rect(Rect{x1: nx, y1: std::cmp::min(py, ny), x2: nx, y2: std::cmp::max(ny, py)}, Tile::empty(), true);
                    }
                }
                rooms.push(new_room);
            }
        }
        let total = width * height; 
        let mut full = 0;
        for y in 0..height-1 {
            for x in 0..width-1 {
                if map.get(x,y).is_wall() {
                    full += 1;
                }
            }
        }
        let percent = full as f64 / total as f64;
        if percent <= 0.6 && percent >= 0.4 {
            break;
        }
    }

    (map, rooms, starting_position)
}

fn handle_camera(state: &mut GameState) {
    if state.player.x - state.camera_pos.0 < -1 {
        state.camera_pos.0 -= 1
    } else if state.player.x - state.camera_pos.0 > 0 {
        state.camera_pos.0 += 1
    }
    if state.player.y - state.camera_pos.1 < -1 {
        state.camera_pos.1 -= 1
    } else if state.player.y - state.camera_pos.1 > 0 {
        state.camera_pos.1 += 1
    }
}

fn compute_fov(state: &mut GameState, force: bool) -> bool {
    if force || state.prev_player_pos != (state.player.x, state.player.y) {
        state.fov_map.compute_fov(state.player.x, state.player.y, TORCH_RADIUS, FOV_LIGHT_WALLS, FOV_ALGO);
        for y in 0..(state.map.height - 1) {
            for x in 0..(state.map.width - 1) {
                let visible = state.fov_map.is_in_fov(x, y);
                let explored = &mut state.map.get_mut(x, y).explored;
                if visible {
                    *explored = true;
                }
            }
        }
        if let Some(id) = state.recent_enemy_id {
            let enemy = &mut state.npcs[id];
            let visible = state.fov_map.is_in_fov(enemy.x, enemy.y);
            if !visible {
                state.recent_enemy_id = None;
            }
        }
        true
    } else {
        false
    }
}

fn render_all(root: &mut Root, con: &mut Offscreen, panel: &mut Offscreen, state: &GameState, rerender_map: bool, messages: &mut Messages) {
    if rerender_map {
        state.map.clear(con);
        state.map.draw(con, (state.player.x, state.player.y), &state.fov_map);
    }
    let mut to_draw: Vec<_> = state.npcs.iter().filter(|o| state.fov_map.is_in_fov(o.x, o.y)).collect();
    for obj in state.objects.iter().filter(|o| state.fov_map.is_in_fov(o.x, o.y)) {
        obj.draw(con);
    }
    to_draw.sort_by(|o1, o2| { o1.blocks.cmp(&o2.blocks) });
    for object in &to_draw {
        object.draw(con);
    }
    state.player.draw(con);
    blit(con, (state.camera_pos.0 - SCREEN_WIDTH / 2, state.camera_pos.1 - SCREEN_HEIGHT / 2), (SCREEN_WIDTH, SCREEN_HEIGHT), root, (0, 0), 1.0, 1.0);

    // prepare to render the GUI panel
    panel.set_default_background(colors::BLACK);
    panel.clear();

    // show the player's stats
    let hp = state.player.fighter.map_or(0, |f| f.hp);
    let max_hp = state.player.fighter.map_or(0, |f| f.max_hp);
    render_bar(panel, 0, 0, BAR_WIDTH, "HP", hp, max_hp, colors::LIGHT_RED, colors::DARKER_RED);

    // how recent enemy's state
    if let Some(id) = state.recent_enemy_id {
        let enemy = &state.npcs[id];
        let hp = enemy.fighter.map_or(0, |f| f.hp);
        let max_hp = enemy.fighter.map_or(0, |f| f.max_hp);
        render_bar(panel, BAR_WIDTH, 0, SCREEN_WIDTH-BAR_WIDTH, format!("{}",enemy.name), hp, max_hp, colors::LIGHT_GREEN, colors::DARKER_GREEN);
    }

    // print the game messages, one line at a time
    let mut y = MSG_HEIGHT as i32;
    for &(ref msg, color) in messages.iter().rev() {
        let msg_height = panel.get_height_rect(MSG_X, 0, MSG_WIDTH, MSG_HEIGHT as i32, msg);
        y -= msg_height;
        if y < 1 {
            break;
        }
        panel.set_default_foreground(color);
        panel.print_rect(MSG_X, y, MSG_WIDTH, 0, msg);
    }

    // blit the contents of `panel` to the root console
    blit(panel, (0, 0), (SCREEN_WIDTH, PANEL_HEIGHT), root, (0, PANEL_Y), 1.0, 1.0);

    // Clear stuff
    root.flush();
    for object in &to_draw {
        object.clear(con);
    }
    for obj in state.objects.iter().filter(|o| state.fov_map.is_in_fov(o.x, o.y)) {
        obj.clear(con);
    }
    state.player.clear(con);
}

fn render_bar<S: Into<String>>(panel: &mut Offscreen,
              x: i32,
              y: i32,
              total_width: i32,
              name: S,
              value: i32,
              maximum: i32,
              bar_color: Color,
              back_color: Color)
{
    let bar_width = (value as f32 / maximum as f32 * total_width as f32) as i32;

    panel.set_default_background(back_color);
    panel.rect(x, y, total_width, 1, false, BackgroundFlag::Screen);

    panel.set_default_background(bar_color);
    if bar_width > 0 {
        panel.rect(x, y, bar_width, 1, false, BackgroundFlag::Screen);
    }

    panel.set_default_foreground(colors::WHITE);
    panel.print_ex(x + total_width / 2, y, BackgroundFlag::None, TextAlignment::Center,
                &format!("{}: {}/{}", name.into(), value, maximum));
}



fn handle_keys(root: &mut Root, con: &mut Offscreen, status: &mut Offscreen, state: &mut GameState, messages: &mut Messages) -> PlayerAction {
    use tcod::input::Key;
    use tcod::input::KeyCode::*;
    use PlayerAction::*;
    use GameStage::*;

    let key = root.wait_for_keypress(true);
    if key.pressed {
        match (key, state.stage) {
            (Key { code: Escape, .. }, _) => return Exit,  // exit game

            // movement keys
            (Key { code: Up, .. }, Playing) => {
                let id = state.player.move_or_attack(0, -1, &mut state.map, &mut state.npcs[..], &mut state.objects[..], state.inventory.has_key, &mut state.stage, messages);
                if let Some(id) = id {
                    state.recent_enemy_id = Some(id);
                }
                return TookTurn;
            },
            (Key { code: Down, .. }, Playing) => {
                let id = state.player.move_or_attack(0, 1, &mut state.map, &mut state.npcs[..], &mut state.objects[..], state.inventory.has_key, &mut state.stage, messages);
                if let Some(id) = id {
                    state.recent_enemy_id = Some(id);
                }
                return TookTurn;
            },
            (Key { code: Left, .. }, Playing) => {
                let id = state.player.move_or_attack(-1, 0, &mut state.map, &mut state.npcs[..], &mut state.objects[..], state.inventory.has_key, &mut state.stage, messages);
                if let Some(id) = id {
                    state.recent_enemy_id = Some(id);
                }
                return TookTurn;
            },
            (Key { code: Right, .. }, Playing) => {
                let id = state.player.move_or_attack(1, 0, &mut state.map, &mut state.npcs[..], &mut state.objects[..], state.inventory.has_key, &mut state.stage, messages);
                if let Some(id) = id {
                    state.recent_enemy_id = Some(id);
                }
                return TookTurn;
            },
            (Key { code: Control, .. }, Playing) => {
                if state.inventory.healing_potions > 0 {
                    state.inventory.healing_potions -= 1;
                    state.player.take_damage(-3, messages);
                    print_message(messages, format!("Used a health potion! You have {} left.", state.inventory.healing_potions), colors::CHARTREUSE);

                } else {
                    print_message(messages, format!("No healing potions left!"), colors::RED);
                }
                return TookTurn;
            },
            (Key { code: Alt, .. }, Playing) => {
                print_message(messages, format!("Waited a turn."), colors::GREY);
                return TookTurn;
            },
            (Key { code: Enter, .. }, Title) => {
                state.stage = Playing;
                messages.clear();
                return TookTurn;
            },
            (Key { code: Enter, .. }, GameOver) => {
                println!("restarting");
                let (initial_map, rooms, (px, py)) = generate_map(MAP_WIDTH-SCREEN_WIDTH-1, MAP_HEIGHT-SCREEN_HEIGHT-1);
                let (px, py) = (px + SCREEN_WIDTH/2, py + SCREEN_HEIGHT/2);

                let mut final_map = Map::new(MAP_WIDTH, MAP_HEIGHT, Tile::wall());

                for y in 0..MAP_HEIGHT-SCREEN_HEIGHT-1 {
                    for x in 0..MAP_WIDTH-SCREEN_WIDTH-1 {
                        final_map.set(x+SCREEN_WIDTH/2, y+SCREEN_HEIGHT/2, initial_map.get(x, y));
                    }
                }

                let npcs = generate_monsters(&rooms[1..], &mut final_map);
                let objects = generate_objects(&rooms[..], &mut final_map);
                let mut player = Entity::new(px, py, '@', colors::WHITE, "James", &mut final_map, true, true).unwrap();
                player.fighter = Some(Fighter{
                    max_hp: 30,
                    hp: 30,
                    defense: 2,
                    power: 5,
                    on_death: DeathCallback::Player,
                });

                state.player = player;
                state.npcs = npcs;
                state.objects = objects;
                state.map = final_map;
                state.camera_pos = (px, py);
                state.fov_map = FovMap::new(MAP_WIDTH, MAP_HEIGHT);
                state.prev_player_pos = (px, py);
                state.stage = GameStage::Title;
                state.recent_enemy_id = None;
                state.inventory = Inventory { healing_potions: 0, has_key: false };

                messages.clear();

                print_message(messages, "New Game Started! Find the key in the Tomb of the Ancient King and bring it back here to unluck the box... or perish. Press Start to Begin!", colors::CYAN);

                // compute initial fov
                for y in 0..MAP_HEIGHT-1 {
                    for x in 0..MAP_WIDTH {
                        state.fov_map.set(x, y,
                            !state.map.get(x, y).blocks_sight,
                            !state.map.get(x, y).blocks_movement
                        );
                    }
                }
                compute_fov(state, true);
                // Render initial state
                root.clear();
                con.clear();
                status.clear();
                render_all(root, con, status, &state, true, messages);
                return TookTurn;
            },
            (Key { code: Enter, .. }, Won) => {
                // RPI GPIO code here
                return Exit;
            },
            _ => {},
        }
    }

    DidntTakeTurn
}
