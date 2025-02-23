/* This file is part of DarkFi (https://dark.fi)
 *
 * Copyright (C) 2020-2023 Dyne.org foundation
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use darkfi::Result;
use darkfi_contract_test_harness::init_logger;

mod harness;
use harness::Harness;

#[async_std::test]
async fn add_blocks() -> Result<()> {
    init_logger();

    // Initialize harness in testing mode
    let th = Harness::new(true).await?;

    // Generate next block
    let block = th.generate_next_block().await?;

    // Add it to nodes
    th.add_blocks(&vec![block]).await?;

    // Validate chains
    th.validate_chains().await?;

    // Thanks for reading
    Ok(())
}
