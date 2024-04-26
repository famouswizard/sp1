use std::{
    env,
    fs::File,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

use crate::witness::GnarkWitness;
use rand::Rng;
use reqwest::{blocking::Client, StatusCode};
use serde::{Deserialize, Serialize};
use sp1_recursion_compiler::{
    constraints::Constraint,
    ir::{Config, Witness},
};
use std::thread;

/// A prover that can generate proofs with the Groth16 protocol using bindings to Gnark.
#[derive(Debug, Clone)]
pub struct Groth16Prover {
    port: String,
}

/// A zero-knowledge proof generated by the Groth16 protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Groth16Proof {
    pub a: [String; 2],
    pub b: [[String; 2]; 2],
    pub c: [String; 2],
    pub public_inputs: [String; 2],
}

impl Groth16Prover {
    /// Starts up the Gnark server using Groth16 on the given port and waits for it to be ready.
    pub fn new() -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let gnark_dir = manifest_dir.join("../gnark");
        let port = env::var("HOST_PORT").unwrap_or_else(|_| generate_random_port().to_string());
        let port_clone = port.clone();

        // Spawn a thread to run the command
        // TODO: version by commit hash instead of by incrementing
        thread::spawn(move || {
            let mut child = Command::new("go")
                .args([
                    "run",
                    "main.go",
                    "serve",
                    "--type",
                    "groth16",
                    "--version",
                    "1",
                    "--port",
                    &port,
                ])
                .current_dir(gnark_dir)
                .stderr(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stdin(Stdio::inherit())
                .spawn()
                .unwrap();

            let exit_status = child.wait().unwrap();

            if !exit_status.success() {
                panic!("Gnark server exited with an error: {:?}", exit_status);
            }
        });

        let prover = Self { port: port_clone };

        prover.wait_for_healthy_server().unwrap();

        prover
    }

    /// Checks if the server is ready to accept requests.
    fn wait_for_healthy_server(&self) -> Result<(), Box<dyn std::error::Error>> {
        let client = Client::new();
        let url = format!("http://localhost:{}/healthz", self.port);

        println!("Waiting for server to be healthy...");

        loop {
            match client.get(&url).send() {
                Ok(response) => {
                    if response.status() == StatusCode::OK {
                        println!("Server is healthy!");
                        return Ok(());
                    } else {
                        println!("Server is not healthy yet: {:?}", response.status());
                    }
                }
                Err(err) => {
                    println!("Server is not healthy yet: {:?}", err);
                }
            }

            thread::sleep(Duration::from_secs(1));
        }
    }

    /// Executes the prover in testing mode with a circuit definition and witness.
    pub fn test<C: Config>(constraints: Vec<Constraint>, witness: Witness<C>) {
        let serialized = serde_json::to_string(&constraints).unwrap();
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let gnark_dir = format!("{}/../gnark", manifest_dir);

        // Write constraints.
        let mut constraints_file = tempfile::NamedTempFile::new().unwrap();
        constraints_file.write_all(serialized.as_bytes()).unwrap();

        // Write witness.
        let mut witness_file = tempfile::NamedTempFile::new().unwrap();
        let gnark_witness = GnarkWitness::new(witness);
        let serialized = serde_json::to_string(&gnark_witness).unwrap();
        witness_file.write_all(serialized.as_bytes()).unwrap();

        // Run `make`.
        let make = Command::new("make")
            .current_dir(&gnark_dir)
            .stderr(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stdin(Stdio::inherit())
            .output()
            .unwrap();
        if !make.status.success() {
            panic!("failed to run make");
        }

        let result = Command::new("go")
            .args([
                "test",
                "-tags=prover_checks",
                "-v",
                "-timeout",
                "100000s",
                "-run",
                "^TestMain$",
                "github.com/succinctlabs/sp1-recursion-gnark",
            ])
            .current_dir(gnark_dir)
            .env("WITNESS_JSON", witness_file.path().to_str().unwrap())
            .env(
                "CONSTRAINTS_JSON",
                constraints_file.path().to_str().unwrap(),
            )
            .stderr(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stdin(Stdio::inherit())
            .output()
            .unwrap();

        if !result.status.success() {
            panic!("failed to run test circuit");
        }
    }

    pub fn build<C: Config>(constraints: Vec<Constraint>, witness: Witness<C>, build_dir: PathBuf) {
        let serialized = serde_json::to_string(&constraints).unwrap();
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let gnark_dir = manifest_dir.join("../gnark");
        let cwd = std::env::current_dir().unwrap();

        // Write constraints.
        let constraints_path = build_dir.join("constraints_groth16.json");
        let mut file = File::create(constraints_path).unwrap();
        file.write_all(serialized.as_bytes()).unwrap();

        // Write witness.
        let witness_path = build_dir.join("witness_groth16.json");
        let gnark_witness = GnarkWitness::new(witness);
        let mut file = File::create(witness_path).unwrap();
        let serialized = serde_json::to_string(&gnark_witness).unwrap();
        file.write_all(serialized.as_bytes()).unwrap();

        // Run `make`.
        let make = Command::new("make")
            .current_dir(&gnark_dir)
            .stderr(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stdin(Stdio::inherit())
            .output()
            .unwrap();
        if !make.status.success() {
            panic!("failed to run make");
        }

        // Run the build script.
        let result = Command::new("go")
            .args([
                "run",
                "main.go",
                "build-groth16",
                "--data",
                cwd.join(build_dir).to_str().unwrap(),
            ])
            .current_dir(gnark_dir)
            .stderr(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stdin(Stdio::inherit())
            .output()
            .unwrap();

        if !result.status.success() {
            panic!("failed to run build script");
        }
    }

    /// Generates a Groth16 proof by sending a request to the Gnark server.
    pub fn prove<C: Config>(&self, witness: Witness<C>) -> Groth16Proof {
        let url = format!("http://localhost:{}/groth16/prove", self.port);
        let response = Client::new().post(&url).json(&witness).send().unwrap();

        // Deserialize the JSON response to a Groth16Proof instance
        let response = response.text().unwrap();
        let proof: Groth16Proof = serde_json::from_str(&response).expect("deserializing the proof");

        proof
    }
}

/// Generate a random port.
fn generate_random_port() -> u16 {
    let mut rng = rand::thread_rng();
    rng.gen_range(1024..49152)
}

impl Default for Groth16Prover {
    fn default() -> Self {
        Self::new()
    }
}
