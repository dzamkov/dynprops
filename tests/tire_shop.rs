//! This example demonstrates how dynamic properties can be used to attach data to an
//! arbitrarily-complex data model without needing to "mirror" the structure of the data model.
#![allow(dead_code)]

/// This module contains code for a vehicle-related data model. Despite the strong tire-related
/// focus for this example, it is not specific to our tire shop.
mod vehicle {
    use dynprops::{Extend, PropertyData};

    pub trait Vehicle {
        fn tires(&self) -> Vec<&Tire>;
    }

    pub struct Motorcycle {
        pub front_tire: Tire,
        pub back_tire: Tire,
    }

    pub struct Car {
        pub front_left_tire: Tire,
        pub front_right_tire: Tire,
        pub back_left_tire: Tire,
        pub back_right_tire: Tire,
    }

    pub struct Truck {
        pub front_left_tire: Tire,
        pub front_right_tire: Tire,
        pub mid_left_tire: DualTire,
        pub mid_right_tire: DualTire,
        pub back_left_tire: DualTire,
        pub back_right_tire: DualTire,
    }

    impl Vehicle for Motorcycle {
        fn tires(&self) -> Vec<&Tire> {
            vec![&self.front_tire, &self.back_tire]
        }
    }

    impl Vehicle for Car {
        fn tires(&self) -> Vec<&Tire> {
            vec![
                &self.front_left_tire,
                &self.front_right_tire,
                &self.back_left_tire,
                &self.back_right_tire,
            ]
        }
    }

    impl Vehicle for Truck {
        fn tires(&self) -> Vec<&Tire> {
            vec![
                &self.front_left_tire,
                &self.front_right_tire,
                &self.mid_left_tire.inner,
                &self.mid_left_tire.outer,
                &self.mid_right_tire.inner,
                &self.mid_right_tire.outer,
                &self.back_left_tire.inner,
                &self.back_left_tire.outer,
                &self.back_right_tire.inner,
                &self.back_right_tire.outer,
            ]
        }
    }

    /// A kind of tire.
    pub struct TireKind {
        pub diameter: f32,
        pub min_pressure: f32,
        pub max_pressure: f32,
        pub cost: f32,
    }

    /// A specific tire on a vehicle.
    #[derive(Extend)]
    pub struct Tire {
        pub kind: &'static TireKind,
        #[prop_data]
        prop_data: PropertyData<Tire>,
    }

    /// A combination of two side-by-side tires, used to increase load capacity on
    /// heavy-duty vehicles.
    pub struct DualTire {
        pub inner: Tire,
        pub outer: Tire,
    }

    /// A standard tire kind for passenger vehicles.
    pub static PASSENGER_33_TIRE: TireKind = TireKind {
        diameter: 33.0,
        min_pressure: 30.0,
        max_pressure: 35.0,
        cost: 160.0,
    };

    /// Creates a new passenger car.
    pub fn new_passenger_car() -> Car {
        Car {
            front_left_tire: Tire {
                kind: &PASSENGER_33_TIRE,
                prop_data: PropertyData::new(),
            },
            front_right_tire: Tire {
                kind: &PASSENGER_33_TIRE,
                prop_data: PropertyData::new(),
            },
            back_left_tire: Tire {
                kind: &PASSENGER_33_TIRE,
                prop_data: PropertyData::new(),
            },
            back_right_tire: Tire {
                kind: &PASSENGER_33_TIRE,
                prop_data: PropertyData::new(),
            },
        }
    }
}

/// This module contains the code used by our tire shop.
mod shop {
    use crate::vehicle::*;
    use dynprops::Property;

    /// The set of observations taken during a tire inspection.
    struct TireCheck {
        pressure: Property<Tire, f32>,
        tread_depth: Property<Tire, f32>,
        notes: Property<Tire, &'static str>,
    }

    /// Gets the estimated cost needed to perform tire-related services on a vehicle.
    fn get_service_cost(vehicle: &dyn Vehicle, check: &TireCheck) -> f32 {
        let mut cost = 0.0;
        let mut need_inflation = false;
        for tire in vehicle.tires() {
            let kind = tire.kind;
            let pressure = *check.pressure.get(tire);
            let tread_depth = *check.tread_depth.get(tire);
            if tread_depth < 4.0 {
                // Needs replacement
                cost += kind.cost;
            } else if pressure < kind.min_pressure {
                // Needs inflation
                need_inflation = true;
                cost += 0.25;
            } else if pressure > kind.max_pressure {
                // Needs deflation
                cost += 0.25;
            }
        }
        if need_inflation {
            // Fixed cost to pull out the air pump
            cost += 2.0;
        }
        return cost;
    }

    #[test]
    fn test_car() {
        // Create car
        let car = new_passenger_car();

        // Take measurements
        let mut pressure = Property::<Tire, f32>::new();
        let mut tread_depth = Property::<Tire, f32>::new();
        let mut notes = Property::<Tire, &'static str>::new();
        pressure.set(&car.front_left_tire, 32.1);
        pressure.set(&car.front_right_tire, 32.3);
        pressure.set(&car.back_left_tire, 28.2);
        pressure.set(&car.back_right_tire, 29.1);
        tread_depth.set(&car.front_left_tire, 4.7);
        tread_depth.set(&car.front_right_tire, 4.3);
        tread_depth.set(&car.back_left_tire, 3.8);
        tread_depth.set(&car.back_right_tire, 4.5);
        notes.set(&car.back_left_tire, "Possible misalignment");
        let check = TireCheck {
            pressure,
            tread_depth,
            notes,
        };

        // Compute service cost
        assert_eq!(get_service_cost(&car, &check), 162.25);

        // Verify notes
        assert_eq!(*check.notes.get(&car.back_right_tire), "");
        assert_eq!(
            *check.notes.get(&car.back_left_tire),
            "Possible misalignment"
        );
    }
}
