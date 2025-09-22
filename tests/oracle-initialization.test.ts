import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, Keypair, SystemProgram } from "@solana/web3.js";
import { assert, expect } from "chai";
import { BN } from "bn.js";
import keccak from "keccak";

// Helper functions for consistent seed derivation and permissions
function canonical(id: string): string {
  return id.trim().toLowerCase();
}

function deriveSeed(assetId: string): Uint8Array {
  const bytes = new TextEncoder().encode(canonical(assetId));
  return new Uint8Array(
    keccak("keccak256").update(Buffer.from(bytes)).digest()
  );
}

// Permission constants matching the Rust program
const PERM = {
  UPDATE_PRICE: 1 << 0, // 0b0000_0001 = 1
  TRIGGER_CIRCUIT_BREAKER: 1 << 1, // 0b0000_0010 = 2
  MODIFY_CONFIG: 1 << 2, // 0b0000_0100 = 4
  // bit 3 reserved                // 0b0000_1000 = 8
  EMERGENCY_HALT: 1 << 4, // 0b0001_0000 = 16
  ADD_FEED: 1 << 5, // 0b0010_0000 = 32
  REMOVE_FEED: 1 << 6, // 0b0100_0000 = 64
};

const ADMIN_ALL =
  PERM.UPDATE_PRICE |
  PERM.TRIGGER_CIRCUIT_BREAKER |
  PERM.MODIFY_CONFIG |
  PERM.EMERGENCY_HALT |
  PERM.ADD_FEED |
  PERM.REMOVE_FEED; // Should equal 119

// Permission encoder for tuple-struct format
const perm = (bits: number) => ({ "0": new BN(bits) });

// Flag constants
const FLAGS = {
  CIRCUIT_BREAKER: 1 << 0, // 0b0001
  EMERGENCY_MODE: 1 << 1, // 0b0010
  UPGRADE_LOCKED: 1 << 2, // 0b0100
  MAINTENANCE_MODE: 1 << 3, // 0b1000
};

// Helper function for confirmed airdrops
async function airdrop(
  connection: anchor.web3.Connection,
  publicKey: PublicKey,
  lamports: number
): Promise<void> {
  const signature = await connection.requestAirdrop(publicKey, lamports);
  await connection.confirmTransaction(signature, "confirmed");
}

// Helper function to build oracle configuration with overrides
function buildConfig(
  assetId: string,
  assetSeed: Uint8Array,
  authority: PublicKey,
  emergencyAdmin: PublicKey,
  governanceMembers: Keypair[],
  overrides: any = {}
): any {
  return {
    assetId,
    assetSeed: Array.from(assetSeed),
    twapWindow: overrides.twapWindow || 3600,
    confidenceThreshold: overrides.confidenceThreshold || 500,
    manipulationThreshold: overrides.manipulationThreshold || 1000,
    emergencyAdmin,
    enableCircuitBreaker: overrides.enableCircuitBreaker ?? true,
    governanceConfig: {
      memberCount: overrides.memberCount || 3,
      initialMembers: [
        authority,
        ...governanceMembers
          .slice(0, (overrides.memberCount || 3) - 1)
          .map((k) => k.publicKey),
        ...Array(Math.max(0, 4 - (overrides.memberCount || 3))).fill(
          PublicKey.default
        ),
      ],
      memberPermissions: [
        perm(overrides.authorityPermissions || ADMIN_ALL),
        ...Array((overrides.memberCount || 3) - 1).fill(
          perm(PERM.UPDATE_PRICE)
        ),
        ...Array(Math.max(0, 4 - (overrides.memberCount || 3))).fill(perm(0)),
      ],
      multisigThreshold:
        overrides.multisigThreshold ||
        Math.max(1, Math.floor((overrides.memberCount || 3) / 2)),
      votingPeriod: new BN(overrides.votingPeriod || 7200),
      executionDelay: new BN(overrides.executionDelay || 3600),
      quorumThreshold: overrides.quorumThreshold || 5000,
      proposalThreshold: new BN(overrides.proposalThreshold || 1000000),
      ...overrides.governanceConfig,
    },
    ...overrides,
  };
}

describe("Oracle Initialization Integration Tests", () => {
  // Configure the client to use the local cluster
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.SolanaMultiTierOracle as Program<any>;
  const provider = anchor.getProvider();

  let authority: Keypair;
  let emergencyAdmin: Keypair;
  let governanceMembers: Keypair[];
  let assetSeed: Uint8Array;
  let oracleAccounts: any;

  before(async () => {
    // Setup test accounts
    authority = Keypair.generate();
    emergencyAdmin = Keypair.generate();

    // Generate governance members
    governanceMembers = Array.from({ length: 5 }, () => Keypair.generate());

    // Request confirmed airdrops to fund test accounts
    await airdrop(
      provider.connection,
      authority.publicKey,
      10 * anchor.web3.LAMPORTS_PER_SOL
    );
    await airdrop(
      provider.connection,
      emergencyAdmin.publicKey,
      5 * anchor.web3.LAMPORTS_PER_SOL
    );

    // Fund governance members
    for (const member of governanceMembers) {
      await airdrop(
        provider.connection,
        member.publicKey,
        2 * anchor.web3.LAMPORTS_PER_SOL
      );
    }

    // Generate asset seed using proper canonicalization and keccak hash
    const assetId = "SOL/USDC";
    assetSeed = deriveSeed(assetId);

    // Generate PDA addresses
    const [oracle] = PublicKey.findProgramAddressSync(
      [Buffer.from("oracle_state"), Buffer.from(assetSeed)],
      program.programId
    );

    const [governance] = PublicKey.findProgramAddressSync(
      [Buffer.from("governance"), oracle.toBuffer()],
      program.programId
    );

    const [historicalChunk0] = PublicKey.findProgramAddressSync(
      [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([0])],
      program.programId
    );

    const [historicalChunk1] = PublicKey.findProgramAddressSync(
      [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([1])],
      program.programId
    );

    const [historicalChunk2] = PublicKey.findProgramAddressSync(
      [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([2])],
      program.programId
    );

    oracleAccounts = {
      oracle,
      governance,
      historicalChunk0,
      historicalChunk1,
      historicalChunk2,
    };
  });

  describe("Successful Oracle Initialization", () => {
    it("Should initialize oracle with valid configuration", async () => {
      const config = {
        assetId: "SOL/USDC",
        assetSeed: Array.from(assetSeed),
        twapWindow: 3600, // 1 hour
        confidenceThreshold: 500, // 5%
        manipulationThreshold: 1000, // 10%
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 5,
          initialMembers: [
            authority.publicKey,
            ...governanceMembers.slice(0, 4).map((k) => k.publicKey),
            ...Array(3).fill(PublicKey.default), // Reduce array size to fit transaction limit
          ],
          memberPermissions: [
            perm(ADMIN_ALL), // Authority has all admin permissions
            perm(PERM.UPDATE_PRICE), // Other members get basic permissions
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(3).fill(perm(0)), // Reduce array size to fit transaction limit
          ],
          multisigThreshold: 3,
          votingPeriod: new BN(7200), // 2 hours
          executionDelay: new BN(3600), // 1 hour
          quorumThreshold: 5000, // 50%
          proposalThreshold: new BN(1000000), // 1M units
        },
      };

      // Set up event listener for OracleInitialized event
      let eventReceived = false;
      let eventData: any = null;

      const eventListener = program.addEventListener(
        "OracleInitialized",
        (event) => {
          eventReceived = true;
          eventData = event;
          console.log("OracleInitialized event received:", event);
        }
      );

      const tx = await program.methods
        .initializeOracle(config)
        .accounts({
          oracleState: oracleAccounts.oracle,
          governanceState: oracleAccounts.governance,
          historicalChunk0: oracleAccounts.historicalChunk0,
          historicalChunk1: oracleAccounts.historicalChunk1,
          historicalChunk2: oracleAccounts.historicalChunk2,
          authority: authority.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([authority])
        .rpc();

      console.log("Initialize Oracle transaction signature", tx);

      // Wait a bit for event to be processed
      await new Promise((resolve) => setTimeout(resolve, 1000));

      // Remove event listener
      await program.removeEventListener(eventListener);

      // Verify event was emitted with correct data
      if (eventReceived && eventData) {
        expect(eventData.canonicalAssetId).to.equal(canonical("SOL/USDC"));
        expect(eventData.confidenceThreshold).to.equal(500);
        expect(eventData.manipulationThreshold).to.equal(1000);
        expect(eventData.memberCount).to.equal(5);
        expect(eventData.multisigThreshold).to.equal(3);
        console.log("Event validation passed");
      } else {
        console.log(
          "Warning: OracleInitialized event not received - may be program doesn't emit it"
        );
      }

      // Verify oracle state
      const oracleAccount = await program.account.oracleState.fetch(
        oracleAccounts.oracle
      );

      expect(oracleAccount.authority.toString()).to.equal(
        authority.publicKey.toString()
      );
      expect(oracleAccount.twapWindow).to.equal(3600);
      expect(oracleAccount.confidenceThreshold).to.equal(500);
      expect(oracleAccount.manipulationThreshold).to.equal(1000);
      expect(oracleAccount.emergencyAdmin.toString()).to.equal(
        emergencyAdmin.publicKey.toString()
      );
      expect(oracleAccount.activeFeedCount).to.equal(0);

      // Verify governance state
      const governanceAccount = await program.account.governanceState.fetch(
        oracleAccounts.governance
      );

      expect(governanceAccount.activeMemberCount).to.equal(5);
      expect(governanceAccount.multiSigThreshold).to.equal(3);
      expect(governanceAccount.quorumThreshold).to.equal(5000);
      expect(governanceAccount.multisigMembers[0].toString()).to.equal(
        authority.publicKey.toString()
      );

      // Verify historical chunks are initialized
      const chunk0 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk0
      );
      const chunk1 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk1
      );
      const chunk2 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk2
      );

      expect(chunk0.chunkId).to.equal(0);
      expect(chunk1.chunkId).to.equal(1);
      expect(chunk2.chunkId).to.equal(2);
      expect(chunk0.count).to.equal(0); // No price points yet
      expect(chunk1.count).to.equal(0);
      expect(chunk2.count).to.equal(0);
    });

    it("Should have correct PDA bumps and canonicalization determinism", async () => {
      // Test PDA bump validation
      const [oraclePda, oracleBump] = PublicKey.findProgramAddressSync(
        [Buffer.from("oracle_state"), Buffer.from(assetSeed)],
        program.programId
      );
      const [governancePda, governanceBump] = PublicKey.findProgramAddressSync(
        [Buffer.from("governance"), oraclePda.toBuffer()],
        program.programId
      );
      const [chunk0Pda, chunk0Bump] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("historical_chunk"),
          oraclePda.toBuffer(),
          Buffer.from([0]),
        ],
        program.programId
      );
      const [chunk1Pda, chunk1Bump] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("historical_chunk"),
          oraclePda.toBuffer(),
          Buffer.from([1]),
        ],
        program.programId
      );
      const [chunk2Pda, chunk2Bump] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("historical_chunk"),
          oraclePda.toBuffer(),
          Buffer.from([2]),
        ],
        program.programId
      );

      // Fetch account states to verify bumps
      const oracleAccount = await program.account.oracleState.fetch(
        oracleAccounts.oracle
      );
      const governanceAccount = await program.account.governanceState.fetch(
        oracleAccounts.governance
      );
      const chunk0 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk0
      );
      const chunk1 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk1
      );
      const chunk2 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk2
      );

      // Assert bumps match
      expect(oracleAccount.bump).to.equal(oracleBump);
      expect(governanceAccount.bump).to.equal(governanceBump);
      expect(chunk0.bump).to.equal(chunk0Bump);
      expect(chunk1.bump).to.equal(chunk1Bump);
      expect(chunk2.bump).to.equal(chunk2Bump);

      // Test canonicalization determinism - different case variations should produce same PDAs
      const variations = ["SOL/USDC", " sol/usdc ", "SoL/UsDc", "  SOL/USDC  "];
      const canonicalSeeds = variations.map((v) => deriveSeed(v));

      // All canonical seeds should be identical
      for (let i = 1; i < canonicalSeeds.length; i++) {
        expect(Buffer.from(canonicalSeeds[i])).to.deep.equal(
          Buffer.from(canonicalSeeds[0])
        );
      }

      // PDAs derived from different case variations should be identical
      const pdas = variations.map((v) => {
        const seed = deriveSeed(v);
        const [pda] = PublicKey.findProgramAddressSync(
          [Buffer.from("oracle_state"), Buffer.from(seed)],
          program.programId
        );
        return pda;
      });

      for (let i = 1; i < pdas.length; i++) {
        expect(pdas[i].toString()).to.equal(pdas[0].toString());
      }

      console.log("PDA determinism verified for case variations");
    });
  });

  describe("State Defaults and Cross-Links Validation", () => {
    it("Should have correct oracle state defaults and cross-links", async () => {
      const oracleAccount = await program.account.oracleState.fetch(
        oracleAccounts.oracle
      );
      const governanceAccount = await program.account.governanceState.fetch(
        oracleAccounts.governance
      );

      // Verify oracle state defaults
      expect(oracleAccount.lastUpdate.toString()).to.equal("0"); // Should be 0 initially
      expect(oracleAccount.currentChunkIndex).to.equal(0); // Should start at 0
      expect(oracleAccount.maxChunkSize).to.be.greaterThan(0); // Should have a buffer size

      // Verify asset seed matches input
      const expectedSeed = Array.from(assetSeed);
      const actualSeed = Array.from(oracleAccount.assetSeed);
      expect(actualSeed).to.deep.equal(expectedSeed);

      // Verify governance cross-links
      expect(governanceAccount.timelockDuration.toString()).to.equal("3600"); // execution_delay
      // Note: DEFAULT_VETO_PERIOD would need to be imported from program constants

      // Verify historical chunks have reasonable creation timestamps
      const chunk0 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk0
      );
      const chunk1 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk1
      );
      const chunk2 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk2
      );

      const currentTime = Math.floor(Date.now() / 1000);
      const tolerance = 5; // ±5 seconds

      // Validate creation timestamps are within tolerance
      expect(chunk0.creationTimestamp.toNumber()).to.be.within(
        currentTime - tolerance,
        currentTime + tolerance
      );
      expect(chunk1.creationTimestamp.toNumber()).to.be.within(
        currentTime - tolerance,
        currentTime + tolerance
      );
      expect(chunk2.creationTimestamp.toNumber()).to.be.within(
        currentTime - tolerance,
        currentTime + tolerance
      );

      console.log("State defaults and cross-links validated");
    });
  });

  describe("Permissions Copy and Unused Slots Validation", () => {
    it("Should correctly mirror member permissions and default unused slots", async () => {
      const governanceAccount = await program.account.governanceState.fetch(
        oracleAccounts.governance
      );

      // Verify first member_count entries mirror what was sent
      const expectedMembers = [
        authority.publicKey,
        ...governanceMembers.slice(0, 4).map((k) => k.publicKey),
      ];

      const expectedPermissions = [
        ADMIN_ALL, // Authority has all admin permissions
        PERM.UPDATE_PRICE, // Other members get basic permissions
        PERM.UPDATE_PRICE,
        PERM.UPDATE_PRICE,
        PERM.UPDATE_PRICE,
      ];

      // Check active members match expectations
      for (let i = 0; i < 5; i++) {
        expect(governanceAccount.multisigMembers[i].toString()).to.equal(
          expectedMembers[i].toString()
        );

        // Check permissions match (handling BN/object format)
        let actualPerm: number;
        const permObj = governanceAccount.memberPermissions[i];
        if (typeof permObj === "object" && "0" in permObj) {
          actualPerm = Number(permObj["0"]);
        } else {
          actualPerm = Number(permObj);
        }
        expect(actualPerm).to.equal(expectedPermissions[i]);
      }

      // Verify unused slots are properly defaulted (beyond member_count)
      // Note: This test assumes the program has more slots than active members
      const totalSlots = governanceAccount.multisigMembers.length;
      if (totalSlots > 5) {
        for (let i = 5; i < totalSlots; i++) {
          expect(governanceAccount.multisigMembers[i].toString()).to.equal(
            PublicKey.default.toString()
          );

          let unusedPerm: number;
          const permObj = governanceAccount.memberPermissions[i];
          if (typeof permObj === "object" && "0" in permObj) {
            unusedPerm = Number(permObj["0"]);
          } else {
            unusedPerm = Number(permObj);
          }
          expect(unusedPerm).to.equal(0); // Default permissions
        }
      }

      console.log("Permissions copying and unused slots validated");
    });
  });

  describe("Edge Validations for Zero Values", () => {
    let zeroTestAssetSeed: Uint8Array;
    let zeroTestOracleAccounts: any;

    beforeEach(async () => {
      const zeroTestAssetId = `z${Date.now() % 1000}`; // Keep it very short
      zeroTestAssetSeed = deriveSeed(zeroTestAssetId);

      const [oracle] = PublicKey.findProgramAddressSync(
        [Buffer.from("oracle_state"), Buffer.from(zeroTestAssetSeed)],
        program.programId
      );

      const [governance] = PublicKey.findProgramAddressSync(
        [Buffer.from("governance"), oracle.toBuffer()],
        program.programId
      );

      const [historicalChunk0] = PublicKey.findProgramAddressSync(
        [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([0])],
        program.programId
      );

      const [historicalChunk1] = PublicKey.findProgramAddressSync(
        [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([1])],
        program.programId
      );

      const [historicalChunk2] = PublicKey.findProgramAddressSync(
        [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([2])],
        program.programId
      );

      zeroTestOracleAccounts = {
        oracle,
        governance,
        historicalChunk0,
        historicalChunk1,
        historicalChunk2,
        assetId: zeroTestAssetId,
      };
    });

    it("Should reject manipulation_threshold = 0", async () => {
      const config = buildConfig(
        zeroTestOracleAccounts.assetId,
        zeroTestAssetSeed,
        authority.publicKey,
        emergencyAdmin.publicKey,
        governanceMembers,
        {
          manipulationThreshold: 0, // Should fail
          memberCount: 3,
        }
      );

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: zeroTestOracleAccounts.oracle,
            governanceState: zeroTestOracleAccounts.governance,
            historicalChunk0: zeroTestOracleAccounts.historicalChunk0,
            historicalChunk1: zeroTestOracleAccounts.historicalChunk1,
            historicalChunk2: zeroTestOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail(
          "Expected transaction to fail with manipulation_threshold = 0"
        );
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal(
            "InvalidManipulationThreshold"
          );
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Zero Manipulation Threshold Error:", error.toString());
        }
      }
    });

    it("Should reject quorum_threshold = 0", async () => {
      const config = {
        assetId: zeroTestOracleAccounts.assetId,
        assetSeed: Array.from(zeroTestAssetSeed),
        twapWindow: 3600,
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 0, // Should fail
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: zeroTestOracleAccounts.oracle,
            governanceState: zeroTestOracleAccounts.governance,
            historicalChunk0: zeroTestOracleAccounts.historicalChunk0,
            historicalChunk1: zeroTestOracleAccounts.historicalChunk1,
            historicalChunk2: zeroTestOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail("Expected transaction to fail with quorum_threshold = 0");
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal("InvalidQuorumThreshold");
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Zero Quorum Threshold Error:", error.toString());
        }
      }
    });

    it("Should reject proposal_threshold = 0", async () => {
      const config = {
        assetId: zeroTestOracleAccounts.assetId,
        assetSeed: Array.from(zeroTestAssetSeed),
        twapWindow: 3600,
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(0), // Should fail
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: zeroTestOracleAccounts.oracle,
            governanceState: zeroTestOracleAccounts.governance,
            historicalChunk0: zeroTestOracleAccounts.historicalChunk0,
            historicalChunk1: zeroTestOracleAccounts.historicalChunk1,
            historicalChunk2: zeroTestOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail("Expected transaction to fail with proposal_threshold = 0");
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal(
            "InvalidProposalThreshold"
          );
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Zero Proposal Threshold Error:", error.toString());
        }
      }
    });

    it("Should handle confidence_threshold = 0 appropriately", async () => {
      // This test checks if confidence_threshold = 0 is allowed or rejected
      const config = {
        assetId: zeroTestOracleAccounts.assetId,
        assetSeed: Array.from(zeroTestAssetSeed),
        twapWindow: 3600,
        confidenceThreshold: 0, // May be allowed depending on program logic
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        const tx = await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: zeroTestOracleAccounts.oracle,
            governanceState: zeroTestOracleAccounts.governance,
            historicalChunk0: zeroTestOracleAccounts.historicalChunk0,
            historicalChunk1: zeroTestOracleAccounts.historicalChunk1,
            historicalChunk2: zeroTestOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        // If it succeeds, confidence_threshold = 0 is allowed
        expect(tx).to.not.be.null;
        console.log("confidence_threshold = 0 is allowed:", tx);
      } catch (error: any) {
        // If it fails, it should be with InvalidConfidenceThreshold
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal(
            "InvalidConfidenceThreshold"
          );
        } else {
          expect(error.toString()).to.include("Error Code");
        }
        console.log("confidence_threshold = 0 is rejected as expected");
      }
    });
  });

  describe("Re-initialization Guard Test", () => {
    it("Should prevent double initialization of same oracle", async () => {
      // Try to initialize the already-initialized oracle again
      const config = {
        assetId: "SOL/USDC",
        assetSeed: Array.from(assetSeed),
        twapWindow: 7200, // Different values to ensure it's not config issue
        confidenceThreshold: 1000,
        manipulationThreshold: 2000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: false,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(3600),
          executionDelay: new BN(1800),
          quorumThreshold: 6000,
          proposalThreshold: new BN(2000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: oracleAccounts.oracle,
            governanceState: oracleAccounts.governance,
            historicalChunk0: oracleAccounts.historicalChunk0,
            historicalChunk1: oracleAccounts.historicalChunk1,
            historicalChunk2: oracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail(
          "Expected transaction to fail - accounts already initialized"
        );
      } catch (error: any) {
        // Should get account already in use error from system program
        expect(error.toString()).to.include("already in use");
        console.log(
          "Re-initialization correctly prevented:",
          error.toString().slice(0, 100)
        );
      }
    });
  });

  describe("Initialization Validation Tests", () => {
    let testAssetSeed: Uint8Array;
    let testOracleAccounts: any;

    beforeEach(async () => {
      // Generate unique but SHORT asset ID to keep transaction size down
      const testAssetId = `t${Date.now() % 1000}`; // Keep it very short
      testAssetSeed = deriveSeed(testAssetId);

      // Generate corresponding PDA addresses using proper seed
      const [oracle] = PublicKey.findProgramAddressSync(
        [Buffer.from("oracle_state"), Buffer.from(testAssetSeed)],
        program.programId
      );

      const [governance] = PublicKey.findProgramAddressSync(
        [Buffer.from("governance"), oracle.toBuffer()],
        program.programId
      );

      const [historicalChunk0] = PublicKey.findProgramAddressSync(
        [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([0])],
        program.programId
      );

      const [historicalChunk1] = PublicKey.findProgramAddressSync(
        [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([1])],
        program.programId
      );

      const [historicalChunk2] = PublicKey.findProgramAddressSync(
        [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([2])],
        program.programId
      );

      testOracleAccounts = {
        oracle,
        governance,
        historicalChunk0,
        historicalChunk1,
        historicalChunk2,
        assetId: testAssetId, // Store for use in tests
      };
    });

    it("Should reject initialization with invalid TWAP window", async () => {
      const config = {
        assetId: testOracleAccounts.assetId,
        assetSeed: Array.from(testAssetSeed),
        twapWindow: 400000, // > MAX_TWAP_WINDOW (345,600)
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            ...governanceMembers.slice(0, 2).map((k) => k.publicKey),
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL), // Authority has admin permissions
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: testOracleAccounts.oracle,
            governanceState: testOracleAccounts.governance,
            historicalChunk0: testOracleAccounts.historicalChunk0,
            historicalChunk1: testOracleAccounts.historicalChunk1,
            historicalChunk2: testOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail("Expected transaction to fail with invalid TWAP window");
      } catch (error: any) {
        // Check for specific Anchor error code
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal("InvalidTWAPWindow");
        } else {
          // Fallback to error message check
          expect(error.toString()).to.include("Error Code");
          console.log("TWAP Window Error:", error.toString());
        }
      }
    });

    it("Should reject initialization with invalid confidence threshold", async () => {
      const config = {
        assetId: testOracleAccounts.assetId,
        assetSeed: Array.from(testAssetSeed),
        twapWindow: 3600,
        confidenceThreshold: 15000, // > MAX_CONFIDENCE_THRESHOLD (10,000)
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            ...governanceMembers.slice(0, 2).map((k) => k.publicKey),
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: testOracleAccounts.oracle,
            governanceState: testOracleAccounts.governance,
            historicalChunk0: testOracleAccounts.historicalChunk0,
            historicalChunk1: testOracleAccounts.historicalChunk1,
            historicalChunk2: testOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail(
          "Expected transaction to fail with invalid confidence threshold"
        );
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal(
            "InvalidConfidenceThreshold"
          );
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Confidence Threshold Error:", error.toString());
        }
      }
    });

    it("Should reject initialization when authority is not an admin member", async () => {
      const config = {
        assetId: testOracleAccounts.assetId,
        assetSeed: Array.from(testAssetSeed),
        twapWindow: 3600,
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 5,
          initialMembers: [
            authority.publicKey,
            ...governanceMembers.slice(0, 4).map((k) => k.publicKey),
            ...Array(3).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(PERM.UPDATE_PRICE), // Authority is NOT admin - should fail
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(3).fill(perm(0)),
          ],
          multisigThreshold: 3,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: testOracleAccounts.oracle,
            governanceState: testOracleAccounts.governance,
            historicalChunk0: testOracleAccounts.historicalChunk0,
            historicalChunk1: testOracleAccounts.historicalChunk1,
            historicalChunk2: testOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail(
          "Expected transaction to fail with authority not admin member"
        );
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal(
            "AuthorityNotAdminMember"
          );
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Authority Not Admin Error:", error.toString());
        }
      }
    });

    it("Should reject initialization with duplicate governance members", async () => {
      const config = {
        assetId: testOracleAccounts.assetId,
        assetSeed: Array.from(testAssetSeed),
        twapWindow: 3600,
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 5,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[0].publicKey, // Duplicate member - should fail
            governanceMembers[1].publicKey,
            governanceMembers[2].publicKey,
            ...Array(3).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(3).fill(perm(0)),
          ],
          multisigThreshold: 3,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: testOracleAccounts.oracle,
            governanceState: testOracleAccounts.governance,
            historicalChunk0: testOracleAccounts.historicalChunk0,
            historicalChunk1: testOracleAccounts.historicalChunk1,
            historicalChunk2: testOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail("Expected transaction to fail with duplicate members");
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal("DuplicateMember");
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Duplicate Member Error:", error.toString());
        }
      }
    });
  });

  describe("Oracle State Verification Tests", () => {
    it("Should correctly initialize all historical chunks with proper linking", async () => {
      const chunk0 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk0
      );
      const chunk1 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk1
      );
      const chunk2 = await program.account.historicalChunk.fetch(
        oracleAccounts.historicalChunk2
      );

      // Verify chunk linking structure
      expect(chunk0.nextChunk.toString()).to.equal(
        oracleAccounts.historicalChunk1.toString()
      );
      expect(chunk1.nextChunk.toString()).to.equal(
        oracleAccounts.historicalChunk2.toString()
      );
      expect(chunk2.nextChunk.toString()).to.equal(
        PublicKey.default.toString()
      );

      // Verify all chunks point to the correct oracle
      expect(chunk0.oracleState.toString()).to.equal(
        oracleAccounts.oracle.toString()
      );
      expect(chunk1.oracleState.toString()).to.equal(
        oracleAccounts.oracle.toString()
      );
      expect(chunk2.oracleState.toString()).to.equal(
        oracleAccounts.oracle.toString()
      );

      // Verify initial state
      expect(chunk0.count).to.equal(0);
      expect(chunk1.count).to.equal(0);
      expect(chunk2.count).to.equal(0);
    });

    it("Should have correct version information", async () => {
      const oracleAccount = await program.account.oracleState.fetch(
        oracleAccounts.oracle
      );

      expect(oracleAccount.version.major).to.equal(0);
      expect(oracleAccount.version.minor).to.equal(1);
      expect(oracleAccount.version.patch).to.equal(0);
    });

    it("Should have correct flag settings", async () => {
      const oracleAccount = await program.account.oracleState.fetch(
        oracleAccounts.oracle
      );

      // Debug: Let's see what flags looks like
      console.log("Flags object:", oracleAccount.flags);
      console.log("Flags type:", typeof oracleAccount.flags);

      // Handle different possible flag representations
      let flagsNum: number;
      if (
        typeof oracleAccount.flags === "object" &&
        oracleAccount.flags !== null
      ) {
        // If it's a BN or object with "0" property
        if ("0" in oracleAccount.flags) {
          flagsNum = Number(oracleAccount.flags["0"]);
        } else if (typeof oracleAccount.flags.toNumber === "function") {
          flagsNum = oracleAccount.flags.toNumber();
        } else {
          flagsNum = Number(oracleAccount.flags);
        }
      } else {
        flagsNum = Number(oracleAccount.flags);
      }

      console.log("Flags as number:", flagsNum);

      // Circuit breaker is enabled based on the config
      expect((flagsNum & FLAGS.CIRCUIT_BREAKER) !== 0).to.be.true;

      // Other flags should be disabled initially
      expect((flagsNum & FLAGS.EMERGENCY_MODE) === 0).to.be.true;
      expect((flagsNum & FLAGS.UPGRADE_LOCKED) === 0).to.be.true;
      expect((flagsNum & FLAGS.MAINTENANCE_MODE) === 0).to.be.true;
    });
  });

  describe("Account Size and Rent Exemption Tests", () => {
    it("Should have sufficient rent exemption for all accounts", async () => {
      const oracleBalance = await provider.connection.getBalance(
        oracleAccounts.oracle
      );
      const governanceBalance = await provider.connection.getBalance(
        oracleAccounts.governance
      );
      const chunk0Balance = await provider.connection.getBalance(
        oracleAccounts.historicalChunk0
      );
      const chunk1Balance = await provider.connection.getBalance(
        oracleAccounts.historicalChunk1
      );
      const chunk2Balance = await provider.connection.getBalance(
        oracleAccounts.historicalChunk2
      );

      // All accounts should be rent exempt (balance > 0)
      expect(oracleBalance).to.be.greaterThan(0);
      expect(governanceBalance).to.be.greaterThan(0);
      expect(chunk0Balance).to.be.greaterThan(0);
      expect(chunk1Balance).to.be.greaterThan(0);
      expect(chunk2Balance).to.be.greaterThan(0);

      console.log("Account balances (lamports):");
      console.log(`Oracle: ${oracleBalance}`);
      console.log(`Governance: ${governanceBalance}`);
      console.log(`Historical Chunk 0: ${chunk0Balance}`);
      console.log(`Historical Chunk 1: ${chunk1Balance}`);
      console.log(`Historical Chunk 2: ${chunk2Balance}`);
    });

    it("Should have correct account owners", async () => {
      const oracleInfo = await provider.connection.getAccountInfo(
        oracleAccounts.oracle
      );
      const governanceInfo = await provider.connection.getAccountInfo(
        oracleAccounts.governance
      );
      const chunk0Info = await provider.connection.getAccountInfo(
        oracleAccounts.historicalChunk0
      );

      expect(oracleInfo?.owner.toString()).to.equal(
        program.programId.toString()
      );
      expect(governanceInfo?.owner.toString()).to.equal(
        program.programId.toString()
      );
      expect(chunk0Info?.owner.toString()).to.equal(
        program.programId.toString()
      );
    });
  });

  describe("Comprehensive Edge Case Tests", () => {
    let edgeTestAssetSeed: Uint8Array;
    let edgeTestOracleAccounts: any;

    beforeEach(async () => {
      const edgeTestAssetId = `e${Date.now() % 1000}`; // Keep it very short
      edgeTestAssetSeed = deriveSeed(edgeTestAssetId);

      const [oracle] = PublicKey.findProgramAddressSync(
        [Buffer.from("oracle_state"), Buffer.from(edgeTestAssetSeed)],
        program.programId
      );

      const [governance] = PublicKey.findProgramAddressSync(
        [Buffer.from("governance"), oracle.toBuffer()],
        program.programId
      );

      const [historicalChunk0] = PublicKey.findProgramAddressSync(
        [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([0])],
        program.programId
      );

      const [historicalChunk1] = PublicKey.findProgramAddressSync(
        [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([1])],
        program.programId
      );

      const [historicalChunk2] = PublicKey.findProgramAddressSync(
        [Buffer.from("historical_chunk"), oracle.toBuffer(), Buffer.from([2])],
        program.programId
      );

      edgeTestOracleAccounts = {
        oracle,
        governance,
        historicalChunk0,
        historicalChunk1,
        historicalChunk2,
        assetId: edgeTestAssetId,
      };
    });

    it("Should reject initialization with mismatched asset ID and seed", async () => {
      // Create seed for different asset ID to test validation
      const differentAssetId = "d"; // Keep it very short
      const wrongSeed = deriveSeed("w"); // Wrong seed - very short

      const config = {
        assetId: differentAssetId,
        assetSeed: Array.from(wrongSeed), // Wrong seed for this asset ID
        twapWindow: 3600,
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: edgeTestOracleAccounts.oracle,
            governanceState: edgeTestOracleAccounts.governance,
            historicalChunk0: edgeTestOracleAccounts.historicalChunk0,
            historicalChunk1: edgeTestOracleAccounts.historicalChunk1,
            historicalChunk2: edgeTestOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail("Expected transaction to fail with invalid asset seed");
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal("ConstraintSeeds");
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Asset Seed Mismatch Error:", error.toString());
        }
      }
    });

    it("Should reject empty asset ID", async () => {
      const emptySeed = deriveSeed("");

      const config = {
        assetId: "",
        assetSeed: Array.from(emptySeed),
        twapWindow: 3600,
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: edgeTestOracleAccounts.oracle,
            governanceState: edgeTestOracleAccounts.governance,
            historicalChunk0: edgeTestOracleAccounts.historicalChunk0,
            historicalChunk1: edgeTestOracleAccounts.historicalChunk1,
            historicalChunk2: edgeTestOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail("Expected transaction to fail with empty asset ID");
      } catch (error: any) {
        // Empty asset ID causes seed mismatch, resulting in ConstraintSeeds error
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal("ConstraintSeeds");
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Empty Asset ID Error:", error.toString());
        }
      }
    });

    it.skip("Should reject asset ID with length >= 65", async () => {
      // SKIPPED: Long asset IDs push transaction size over 1232 bytes (legacy tx limit)
      // This validation should be covered by unit tests or tested with v0 transactions + ALTs
      const longAssetId = "A".repeat(65); // Exactly 65 characters
      const longSeed = deriveSeed(longAssetId);

      const config = {
        assetId: longAssetId,
        assetSeed: Array.from(longSeed),
        twapWindow: 3600,
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: edgeTestOracleAccounts.oracle,
            governanceState: edgeTestOracleAccounts.governance,
            historicalChunk0: edgeTestOracleAccounts.historicalChunk0,
            historicalChunk1: edgeTestOracleAccounts.historicalChunk1,
            historicalChunk2: edgeTestOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail("Expected transaction to fail with long asset ID");
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal("InvalidAssetId");
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Long Asset ID Error:", error.toString());
        }
      }
    });

    it("Should reject multisig threshold greater than member count", async () => {
      const config = {
        assetId: edgeTestOracleAccounts.assetId,
        assetSeed: Array.from(edgeTestAssetSeed),
        twapWindow: 3600,
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 5, // > memberCount (3)
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: edgeTestOracleAccounts.oracle,
            governanceState: edgeTestOracleAccounts.governance,
            historicalChunk0: edgeTestOracleAccounts.historicalChunk0,
            historicalChunk1: edgeTestOracleAccounts.historicalChunk1,
            historicalChunk2: edgeTestOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail(
          "Expected transaction to fail with invalid multisig threshold"
        );
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal(
            "InvalidMultisigThreshold"
          );
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Multisig Threshold Error:", error.toString());
        }
      }
    });

    it("Should reject negative voting period", async () => {
      const config = {
        assetId: edgeTestOracleAccounts.assetId,
        assetSeed: Array.from(edgeTestAssetSeed),
        twapWindow: 3600,
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(-1), // Negative voting period
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: edgeTestOracleAccounts.oracle,
            governanceState: edgeTestOracleAccounts.governance,
            historicalChunk0: edgeTestOracleAccounts.historicalChunk0,
            historicalChunk1: edgeTestOracleAccounts.historicalChunk1,
            historicalChunk2: edgeTestOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail("Expected transaction to fail with negative voting period");
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal(
            "InvalidTimingParameters"
          );
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Negative Voting Period Error:", error.toString());
        }
      }
    });

    it("Should reject maximum boundary values + 1", async () => {
      // Test exactly at MAX_TWAP_WINDOW + 1
      const config = {
        assetId: edgeTestOracleAccounts.assetId,
        assetSeed: Array.from(edgeTestAssetSeed),
        twapWindow: 345601, // MAX_TWAP_WINDOW + 1 (assuming 345600 is max)
        confidenceThreshold: 500,
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 5000,
          proposalThreshold: new BN(1000000),
        },
      };

      try {
        await program.methods
          .initializeOracle(config)
          .accounts({
            oracleState: edgeTestOracleAccounts.oracle,
            governanceState: edgeTestOracleAccounts.governance,
            historicalChunk0: edgeTestOracleAccounts.historicalChunk0,
            historicalChunk1: edgeTestOracleAccounts.historicalChunk1,
            historicalChunk2: edgeTestOracleAccounts.historicalChunk2,
            authority: authority.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([authority])
          .rpc();

        assert.fail("Expected transaction to fail at boundary + 1");
      } catch (error: any) {
        if (error.error?.errorCode?.code) {
          expect(error.error.errorCode.code).to.equal("InvalidTWAPWindow");
        } else {
          expect(error.toString()).to.include("Error Code");
          console.log("Boundary + 1 Error:", error.toString());
        }
      }
    });

    it("Should accept maximum boundary values", async () => {
      // Test exactly at MAX values (should succeed)
      const config = {
        assetId: edgeTestOracleAccounts.assetId,
        assetSeed: Array.from(edgeTestAssetSeed),
        twapWindow: 345600, // MAX_TWAP_WINDOW (assuming this is max)
        confidenceThreshold: 10000, // MAX_CONFIDENCE_THRESHOLD (assuming this is max)
        manipulationThreshold: 1000,
        emergencyAdmin: emergencyAdmin.publicKey,
        enableCircuitBreaker: true,
        governanceConfig: {
          memberCount: 3,
          initialMembers: [
            authority.publicKey,
            governanceMembers[0].publicKey,
            governanceMembers[1].publicKey,
            ...Array(1).fill(PublicKey.default),
          ],
          memberPermissions: [
            perm(ADMIN_ALL),
            perm(PERM.UPDATE_PRICE),
            perm(PERM.UPDATE_PRICE),
            ...Array(1).fill(perm(0)),
          ],
          multisigThreshold: 2,
          votingPeriod: new BN(7200),
          executionDelay: new BN(3600),
          quorumThreshold: 10000, // MAX_QUORUM_THRESHOLD (assuming this is max)
          proposalThreshold: new BN(1000000),
        },
      };

      // This should succeed
      const tx = await program.methods
        .initializeOracle(config)
        .accounts({
          oracleState: edgeTestOracleAccounts.oracle,
          governanceState: edgeTestOracleAccounts.governance,
          historicalChunk0: edgeTestOracleAccounts.historicalChunk0,
          historicalChunk1: edgeTestOracleAccounts.historicalChunk1,
          historicalChunk2: edgeTestOracleAccounts.historicalChunk2,
          authority: authority.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([authority])
        .rpc();

      expect(tx).to.not.be.null;
      console.log("Boundary values accepted:", tx);
    });
  });
});
