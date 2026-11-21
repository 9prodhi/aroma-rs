import java.io.BufferedReader;
import java.io.FileReader;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;

class ReadLinesFilter {
    List<String> readMatching(String path, String prefix) throws IOException {
        BufferedReader reader = new BufferedReader(new FileReader(path));
        List<String> matches = new ArrayList<>();
        String line = reader.readLine();
        while (line != null) {
            if (line.startsWith(prefix)) {
                matches.add(line);
            }
            line = reader.readLine();
        }
        reader.close();
        return matches;
    }
}
