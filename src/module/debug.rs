use crate::error::Result;
use crate::module::Module;
use gimli::LittleEndian;

type Dwarf<'a> = gimli::read::Dwarf<gimli::read::EndianSlice<'a, LittleEndian>>;

impl Module {
    pub(crate) fn parse_debug_sections(&mut self, sections: &[(&str, &[u8])]) -> Result<()> {
        log::info!("parsing {} debug sections", sections.len());
        let mut dwarf = Dwarf::default();
        let mut ranges = None;
        let rnglists = None;
        for (name, data) in sections {
            match *name {
                ".debug_info" => {
                    dwarf.debug_info = gimli::read::DebugInfo::new(data, LittleEndian);
                }
                ".debug_ranges" => {
                    ranges = Some(gimli::read::DebugRanges::new(data, LittleEndian));
                }
                ".debug_abbrev" => {
                    dwarf.debug_abbrev = gimli::read::DebugAbbrev::new(data, LittleEndian);
                }
                ".debug_line" => {
                    dwarf.debug_line = gimli::read::DebugLine::new(data, LittleEndian);
                }
                ".debug_str" => {
                    dwarf.debug_str = gimli::read::DebugStr::new(data, LittleEndian);
                }
                _ => {
                    log::debug!("skipping debug section {}", name);
                }
            }
        }
        let debug_ranges = ranges.unwrap_or_default();
        let debug_rnglists = rnglists.unwrap_or_default();
        dwarf.ranges = gimli::read::RangeLists::new(debug_ranges, debug_rnglists);

        let mut iter = dwarf.units();
        while let Some(header) = iter.next()? {
            let unit = dwarf.unit(header)?;
            println!("===================== Unit ====================");
            println!("comp dir: {:?}", unit.comp_dir.as_ref().unwrap().to_string());
            println!("name: {:?}", unit.name.as_ref().unwrap().to_string());
            println!("low pc: 0x{:x}", unit.low_pc);
            // println!("addr base: {:?}", dwarf.address(&unit, unit.addr_base));
            let unit = dwarf.unit(header)?;
            if let Some(program) = unit.line_program.clone() {
                println!("line range: {}", program.header().line_range());
                println!("line base: {}", program.header().line_base());
                let mut rows = program.rows();
                while let Some((header, row)) = rows.next_row()? {
                    let line = row.line().unwrap_or(0);
                    let col = match row.column() {
                        gimli::read::ColumnType::Column(x) => x,
                        gimli::read::ColumnType::LeftEdge => 0,
                    };
                    let file = match row.file(header) {
                        Some(file) => {
                            let name = dwarf.attr_string(&unit, file.path_name())?
                                .to_string_lossy();
                            match file.directory(header) {
                                Some(dir) => {
                                    let dir = dwarf.attr_string(&unit, dir)?
                                        .to_string_lossy();
                                    format!("{}/{}", dir, name)
                                }
                                None => name.to_string(),
                            }
                        }
                        None => String::new()
                    };
                    println!("\t0x{:08x} {}:{}:{}", row.address(), file, line, col);
                }
            }

            let mut entries = unit.entries();
            while let Some((i, entry)) = entries.next_dfs()? {
                println!("entry {} ======================",i);
                match entry.tag() {
                    gimli::DW_TAG_subprogram => {
                        println!("DW_TAG_subprogram");
                    }
                    gimli::DW_TAG_namespace => {
                        println!("DW_TAG_namespace");
                    }
                    gimli::DW_TAG_compile_unit => {
                        println!("DW_TAG_compile_unit");
                    }
                    _ => println!("tag: {:?}", entry.tag()),
                }
				let mut attrs = entry.attrs();
				while let Some(attr) = attrs.next().unwrap() {
					print!("{}=", attr.name().static_string().unwrap());
                    if let Some(s) = attr.string_value(&dwarf.debug_str) {
                        println!("{}", s.to_string().unwrap());
                    } else {
                        println!("{:?}", attr.value());
                    }
				}
            }
        }

        // match name {
        //     ".debug_info" => self.parse_debug_info_section(payload),
        //     _ => {
        //     }
        // }
        Ok(())
    }
}
