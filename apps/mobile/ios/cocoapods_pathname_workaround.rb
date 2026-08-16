# CocoaPods intermittently raises "pathname contains null byte" while resolving
# pnpm-backed local React Native pods. Avoid resolving those symlinks and keep
# CocoaPods' existing relative group layout instead.
# https://github.com/CocoaPods/CocoaPods/issues/12798#issuecomment-4363087583
module CocoaPodsPathnameWorkaround
  def group_for_path_in_group(absolute_pathname, group, reflect_file_system_structure, base_path = nil)
    raise ArgumentError, "Paths must be absolute #{absolute_pathname}" unless absolute_pathname.absolute?
    raise ArgumentError, "Paths must be absolute #{base_path}" unless base_path.nil? || base_path.absolute?

    relative_base = base_path.nil? ? group.real_path : base_path.cleanpath
    relative_pathname = absolute_pathname.relative_path_from(relative_base)
    relative_dir = relative_pathname.dirname

    if reflect_file_system_structure
      path = relative_base
      relative_dir.each_filename do |name|
        break if name.to_s.downcase.include?('.lproj')
        next if name == '.'

        path += name
        group = group.children.find { |child| child.display_name == name } || group.new_group(name, path)
      end
      return group
    end

    group
  end
end

Pod::Project.prepend(CocoaPodsPathnameWorkaround)
