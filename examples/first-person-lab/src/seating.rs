//! Local, advisory seating. Peer poses are not authoritative reservations.
use crate::{FirstPersonController, FirstPersonLab, theater, view::ViewRig};

#[derive(Default)]
pub(crate) enum Seating {
    #[default]
    Standing,
    Seated {
        index: u8,
        saved_view: ViewRig,
    },
}

impl Seating {
    pub(crate) fn index(&self) -> Option<u8> {
        match self {
            Self::Standing => None,
            Self::Seated { index, .. } => Some(*index),
        }
    }
}

impl FirstPersonLab {
    pub(crate) fn occupied_seats(&self) -> [bool; theater::SEAT_COUNT] {
        let mut occupied = self.npcs.occupied_seats();
        for player in self.remote_players.values() {
            if let Some(index) = player.seat {
                occupied[usize::from(index)] = true;
            }
        }
        occupied
    }

    /// Actual players only: never feed NPC claims back into their own contention input.
    pub(crate) fn player_occupied_seats(&self) -> [bool; theater::SEAT_COUNT] {
        let mut occupied = [false; theater::SEAT_COUNT];
        for player in self.remote_players.values() {
            if let Some(index) = player.seat {
                occupied[usize::from(index)] = true;
            }
        }
        if let Some(index) = self.seating.index() {
            occupied[usize::from(index)] = true;
        }
        occupied
    }

    fn sync_controller_direction(&mut self) {
        let old = self.controller;
        self.controller = FirstPersonController::from_camera(&self.view.player);
        self.controller.move_speed = old.move_speed;
        self.controller.sprint_multiplier = old.sprint_multiplier;
        self.controller.look_sensitivity = old.look_sensitivity;
        self.controller.bounds = old.bounds;
    }

    pub(crate) fn apply_seat_request(&mut self, target: u32) -> bool {
        if target == 200 {
            return self.exit_seat();
        }
        let Some(index) = target
            .checked_sub(100)
            .filter(|index| *index < theater::SEAT_COUNT as u32)
        else {
            return false;
        };
        let seat = theater::SEATS[index as usize];
        if self.seating.index().is_some()
            || self.occupied_seats()[index as usize]
            || !self.view.player.eye.is_finite()
            || self.view.player.eye.distance(seat.seated_eye()) > 2.5
            || !theater::has_line_of_sight(self.view.player.eye, seat.seated_eye(), &self.colliders)
            || !theater::npc_path_clear(seat.approach(), seat.approach(), &self.colliders)
        {
            return false;
        }
        self.seating = Seating::Seated {
            index: index as u8,
            saved_view: self.view.clone(),
        };
        self.view.force_first_person();
        self.view.player.eye = seat.seated_eye();
        self.view.player.target = theater::SCREEN_TARGET;
        self.sync_controller_direction();
        self.next_publish_at = 0.0;
        true
    }

    pub(crate) fn exit_seat(&mut self) -> bool {
        let Seating::Seated { index, saved_view } = &self.seating else {
            return false;
        };
        let exit = theater::SEATS[usize::from(*index)].approach();
        // Validate the destination, not a sweep starting inside the chair collider.
        if !theater::npc_path_clear(exit, exit, &self.colliders) {
            return false;
        }
        let direction = (self.view.player.target - self.view.player.eye).normalize_or_zero();
        self.view = saved_view.clone();
        self.view.player.eye = exit;
        self.view.player.target = exit + direction;
        self.seating = Seating::Standing;
        self.sync_controller_direction();
        self.seat_wait_for_neutral = true;
        self.scene.camera = self.view.camera(0.0, &self.colliders);
        self.next_publish_at = 0.0;
        true
    }

    pub(crate) fn resolve_seat_conflict(&mut self) {
        if let (Some(local), Some(seat)) = (self.local_entity_id, self.seating.index())
            && self
                .remote_players
                .iter()
                .any(|(id, player)| *id < local && player.seat == Some(seat))
        {
            self.exit_seat();
        }
    }
}
