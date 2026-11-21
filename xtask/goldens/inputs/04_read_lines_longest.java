import java.io.BufferedReader;
import java.io.FileReader;
import java.io.IOException;

class ReadLinesLongest {
    String longestLine(String path) throws IOException {
        BufferedReader reader = new BufferedReader(new FileReader(path));
        String longest = "";
        String line = reader.readLine();
        while (line != null) {
            if (line.length() > longest.length()) {
                longest = line;
            }
            line = reader.readLine();
        }
        reader.close();
        return longest;
    }
}
